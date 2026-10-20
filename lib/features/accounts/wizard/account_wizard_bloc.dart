import 'dart:async';

import 'package:flutter_bloc/flutter_bloc.dart';
import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/logging/log.dart';
import 'package:mailslate/core/system/url_opener.dart';

/// Which provider card the user picked (USER_FLOW §3 step 2).
enum WizardProvider { gmail, outlook, otherImap }

/// Drives the account onboarding wizard (USER_FLOW §3).
///
/// The bloc never imports generated bindings: it talks to the daemon through
/// [FfiClient] and consumes daemon events through the injected [AppEvent]
/// stream — the selective subscription ARCHITECTURE §4 prescribes for
/// feature blocs.
class AccountWizardBloc extends Bloc<AccountWizardEvent, AccountWizardState> {
  /// Creates the wizard for [firstAccount] (welcome step) or an additional
  /// account (straight to the provider pick).
  AccountWizardBloc(
    this._client,
    this._urlOpener, {
    required Stream<AppEvent> events,
    required bool firstAccount,
    Duration oauthDeadline = const Duration(minutes: 2),
  }) : _oauthTimeout = oauthDeadline,
       super(firstAccount ? const WizardWelcome() : const ChooseProvider()) {
    _events = events.listen(_onAppEvent);
    on<ConnectAccountRequested>(_onConnectAccountRequested);
    on<ProviderPicked>(_onProviderPicked);
    on<ReopenBrowserRequested>(_onReopenBrowser);
    on<OauthRetryRequested>(_onOauthRetry);
    on<BackToProviderPickRequested>(_onBackToProviderPick);
    on<ImapConnectionTestRequested>(_onImapConnectionTest);
    on<ImapAccountSaveRequested>(_onImapAccountSave);
    on<_OauthTimerFired>(_onOauthTimerFired);
    on<_OauthFinished>(_onOauthFinished);
    on<_OauthRefused>(_onOauthRefused);
  }

  final FfiClient _client;
  final UrlOpener _urlOpener;
  final Duration _oauthTimeout;

  StreamSubscription<AppEvent>? _events;
  Timer? _oauthTimer;

  @override
  Future<void> close() async {
    _oauthTimer?.cancel();
    await _events?.cancel();
    return super.close();
  }

  Future<void> _onConnectAccountRequested(
    ConnectAccountRequested event,
    Emitter<AccountWizardState> emit,
  ) async {
    if (state is WizardWelcome) {
      emit(const ChooseProvider());
    }
  }

  Future<void> _onProviderPicked(
    ProviderPicked event,
    Emitter<AccountWizardState> emit,
  ) async {
    if (event.provider == WizardProvider.otherImap) {
      emit(const EnterImapDetails());
      return;
    }
    await _startOauth(event.provider, emit);
  }

  Future<void> _startOauth(
    WizardProvider provider,
    Emitter<AccountWizardState> emit,
  ) async {
    try {
      final url = await _client.startOauth(provider: _daemonProvider(provider));
      if (emit.isDone) return;
      await _urlOpener.open(url);
      if (emit.isDone) return;
      emit(WaitingForOauth(provider: provider, url: url));
      _startOauthTimer();
    } on AppFailure catch (failure) {
      if (emit.isDone) return;
      emit(ChooseProvider(note: failure.message));
    } catch (error, stackTrace) {
      if (emit.isDone) return;
      logError(
        'wizard_oauth_start_failed',
        error: error,
        stackTrace: stackTrace,
      );
      emit(const ChooseProvider(note: 'Something went wrong — try again'));
    }
  }

  void _startOauthTimer() {
    _oauthTimer?.cancel();
    _oauthTimer = Timer(_oauthTimeout, () {
      if (!isClosed) {
        add(const _OauthTimerFired());
      }
    });
  }

  static String _daemonProvider(WizardProvider provider) => switch (provider) {
    WizardProvider.gmail => 'google',
    WizardProvider.outlook => 'microsoft',
    WizardProvider.otherImap => 'other',
  };

  Future<void> _onReopenBrowser(
    ReopenBrowserRequested event,
    Emitter<AccountWizardState> emit,
  ) async {
    final state = this.state;
    if (state is! WaitingForOauth) return;
    await _urlOpener.open(state.url);
  }

  Future<void> _onOauthRetry(
    OauthRetryRequested event,
    Emitter<AccountWizardState> emit,
  ) async {
    final state = this.state;
    if (state is! OauthTimedOut) return;
    await _startOauth(state.provider, emit);
  }

  Future<void> _onBackToProviderPick(
    BackToProviderPickRequested event,
    Emitter<AccountWizardState> emit,
  ) async {
    _oauthTimer?.cancel();
    emit(const ChooseProvider());
  }

  Future<void> _onOauthTimerFired(
    _OauthTimerFired event,
    Emitter<AccountWizardState> emit,
  ) async {
    final state = this.state;
    if (state is! WaitingForOauth) return;
    emit(OauthTimedOut(provider: state.provider, url: state.url));
  }

  Future<void> _onOauthFinished(
    _OauthFinished event,
    Emitter<AccountWizardState> emit,
  ) async {
    final state = this.state;
    if (state is! WaitingForOauth) return;
    _oauthTimer?.cancel();
    emit(const AddingAccount());
    try {
      final accountId = await _client.completeOauthAccount(
        // USER_FLOW §3 has no name step: the address is the display name.
        displayName: event.email,
        email: event.email,
      );
      if (emit.isDone) return;
      emit(AccountConnected(accountId: accountId));
    } on AppFailure catch (failure) {
      if (emit.isDone) return;
      emit(ChooseProvider(note: failure.message));
    } catch (error, stackTrace) {
      if (emit.isDone) return;
      logError(
        'wizard_oauth_finish_failed',
        error: error,
        stackTrace: stackTrace,
      );
      emit(const ChooseProvider(note: 'Something went wrong — try again'));
    }
  }

  Future<void> _onOauthRefused(
    _OauthRefused event,
    Emitter<AccountWizardState> emit,
  ) async {
    final state = this.state;
    if (state is! WaitingForOauth) return;
    _oauthTimer?.cancel();
    emit(ChooseProvider(note: event.reason));
  }

  Future<void> _onImapConnectionTest(
    ImapConnectionTestRequested event,
    Emitter<AccountWizardState> emit,
  ) async {
    emit(const TestingImapConnection());
    try {
      final result = await _client.testConnection(
        draft: event.draft,
        password: event.password,
      );
      if (emit.isDone) return;
      emit(EnterImapDetails(lastTest: result));
    } on AppFailure catch (failure) {
      if (emit.isDone) return;
      emit(EnterImapDetails(note: failure.message));
    } catch (error, stackTrace) {
      if (emit.isDone) return;
      logError('wizard_imap_test_failed', error: error, stackTrace: stackTrace);
      emit(const EnterImapDetails(note: 'Something went wrong — try again'));
    }
  }

  Future<void> _onImapAccountSave(
    ImapAccountSaveRequested event,
    Emitter<AccountWizardState> emit,
  ) async {
    final previous = state is EnterImapDetails
        ? (state as EnterImapDetails).lastTest
        : null;
    emit(const AddingAccount());
    try {
      final accountId = await _client.addImapAccount(
        draft: event.draft,
        password: event.password,
      );
      if (emit.isDone) return;
      emit(AccountConnected(accountId: accountId));
    } on AppFailure catch (failure) {
      if (emit.isDone) return;
      emit(EnterImapDetails(lastTest: previous, note: failure.message));
    } catch (error, stackTrace) {
      if (emit.isDone) return;
      logError('wizard_imap_save_failed', error: error, stackTrace: stackTrace);
      emit(
        EnterImapDetails(
          lastTest: previous,
          note: 'Something went wrong — try again',
        ),
      );
    }
  }

  void _onAppEvent(AppEvent event) {
    final state = this.state;
    if (state is! WaitingForOauth) return;
    switch (event) {
      case AppEvent_OauthCompleted(:final email):
        add(_OauthFinished(email: email));
      case AppEvent_OauthFailed(:final reason):
        add(_OauthRefused(reason: reason));
      case _:
        break;
    }
  }
}

/// A step the user takes in the wizard.
sealed class AccountWizardEvent {
  const AccountWizardEvent();
}

/// The user tapped `Connect account` on the welcome step.
final class ConnectAccountRequested extends AccountWizardEvent {
  const ConnectAccountRequested();
}

/// The user chose a provider card.
final class ProviderPicked extends AccountWizardEvent {
  const ProviderPicked({required this.provider});

  /// The card that was tapped.
  final WizardProvider provider;
}

/// The user tapped `Reopen browser` on the OAuth waiting step.
final class ReopenBrowserRequested extends AccountWizardEvent {
  const ReopenBrowserRequested();
}

/// The user tapped `Try again` after the loopback timeout.
final class OauthRetryRequested extends AccountWizardEvent {
  const OauthRetryRequested();
}

/// The user walked back to the provider cards.
final class BackToProviderPickRequested extends AccountWizardEvent {
  const BackToProviderPickRequested();
}

/// The user asked to probe the entered IMAP/SMTP settings before saving.
final class ImapConnectionTestRequested extends AccountWizardEvent {
  const ImapConnectionTestRequested({
    required this.draft,
    required this.password,
  });

  /// The settings as entered in the form.
  final ImapDraft draft;

  /// The password as entered in the form.
  final String password;
}

/// The user asked to save the entered IMAP account.
final class ImapAccountSaveRequested extends AccountWizardEvent {
  const ImapAccountSaveRequested({required this.draft, required this.password});

  /// The settings as entered in the form.
  final ImapDraft draft;

  /// The password as entered in the form.
  final String password;
}

final class _OauthTimerFired extends AccountWizardEvent {
  const _OauthTimerFired();
}

final class _OauthFinished extends AccountWizardEvent {
  const _OauthFinished({required this.email});

  /// The address the provider reported at consent.
  final String email;
}

final class _OauthRefused extends AccountWizardEvent {
  const _OauthRefused({required this.reason});

  /// Why the OAuth flow failed.
  final String reason;
}

/// Which wizard step is showing.
sealed class AccountWizardState {
  const AccountWizardState();
}

/// First-run welcome (USER_FLOW §3 step 1).
final class WizardWelcome extends AccountWizardState {
  const WizardWelcome();

  @override
  bool operator ==(Object other) => other is WizardWelcome;

  @override
  int get hashCode => runtimeType.hashCode;
}

/// The three provider cards, with an optional inline note.
final class ChooseProvider extends AccountWizardState {
  const ChooseProvider({this.note});

  /// Inline note shown above the cards (denied consent, failed start).
  final String? note;

  @override
  bool operator ==(Object other) =>
      other is ChooseProvider && other.note == note;

  @override
  int get hashCode => note.hashCode;
}

/// Waiting for the browser round-trip (USER_FLOW §3 step 3).
final class WaitingForOauth extends AccountWizardState {
  const WaitingForOauth({required this.provider, required this.url});

  /// The provider being authorized.
  final WizardProvider provider;

  /// The authorization URL opened in the browser.
  final String url;

  @override
  bool operator ==(Object other) =>
      other is WaitingForOauth &&
      other.provider == provider &&
      other.url == url;

  @override
  int get hashCode => Object.hash(provider, url);
}

/// The loopback listener timed out (2 minutes).
final class OauthTimedOut extends AccountWizardState {
  const OauthTimedOut({required this.provider, required this.url});

  /// The provider whose flow timed out.
  final WizardProvider provider;

  /// The authorization URL, re-openable via `Try again`.
  final String url;

  @override
  bool operator ==(Object other) =>
      other is OauthTimedOut && other.provider == provider && other.url == url;

  @override
  int get hashCode => Object.hash(provider, url);
}

/// The Other IMAP details form (USER_FLOW §3 step 4).
final class EnterImapDetails extends AccountWizardState {
  const EnterImapDetails({this.lastTest, this.note});

  /// The latest probe results, once `Test connection` ran.
  final ConnectionTest? lastTest;

  /// Inline note (probe failure, save failure).
  final String? note;

  @override
  bool operator ==(Object other) =>
      other is EnterImapDetails &&
      other.lastTest == lastTest &&
      other.note == note;

  @override
  int get hashCode => Object.hash(lastTest, note);
}

/// The IMAP/SMTP probe is in flight.
final class TestingImapConnection extends AccountWizardState {
  const TestingImapConnection();

  @override
  bool operator ==(Object other) => other is TestingImapConnection;

  @override
  int get hashCode => runtimeType.hashCode;
}

/// An account is being written on the daemon.
final class AddingAccount extends AccountWizardState {
  const AddingAccount();

  @override
  bool operator ==(Object other) => other is AddingAccount;

  @override
  int get hashCode => runtimeType.hashCode;
}

/// The account exists on the daemon and initial sync can start.
final class AccountConnected extends AccountWizardState {
  const AccountConnected({required this.accountId});

  /// The daemon's row id for the new account.
  final int accountId;

  @override
  bool operator ==(Object other) =>
      other is AccountConnected && other.accountId == accountId;

  @override
  int get hashCode => accountId.hashCode;
}
