import 'dart:async';

import 'package:flutter_bloc/flutter_bloc.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/logging/log.dart';
import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/core/system/url_opener.dart';
import 'package:mailslate/features/shell/app_events_bloc.dart';

/// Sidebar accounts surface: the account list (F1.5), the inline
/// Reconnect card for expired OAuth grants (F1.6), and the reload
/// signals that keep both in step with the daemon (ARCHITECTURE §4).
///
/// The bloc owns one [FfiClient.listAccounts] snapshot and a set of
/// account ids whose sync actor reported `reauth_required`. Daemon
/// events arrive through the app-wide [AppEventsBloc] state stream the
/// shell subscribes this bloc to, so the daemon connection stays single
/// (PERFORMANCE_GUIDELINES §4.5: unrelated events trigger no work).
///
/// Only one reconnect flow can be pending at a time — the daemon
/// rejects a second OAuth session while one waits — so the pending
/// account id is enough to attribute later `oauth_reconnected` /
/// `oauth_failed` events to the card that started them.
class AccountsBloc extends Bloc<AccountsEvent, AccountsState> {
  /// Subscribes to [events]; the subscription lives until [close].
  ///
  /// [AppEventsBloc] wraps every daemon event in [AppEventsLive]; other
  /// states are ignored.
  AccountsBloc(this._client, Stream<AppEventsState> events, this._urlOpener)
    : super(const AccountsState()) {
    on<AccountsLoadRequested>(_onLoadRequested);
    on<AccountsEventReceived>(_onEventReceived);
    on<AccountReconnectRequested>(_onReconnectRequested);
    _subscription = events.listen(
      (state) {
        if (state is AppEventsLive) {
          add(AccountsEventReceived(state.event));
        }
      },
      onError: (Object error, StackTrace stackTrace) => logError(
        'accounts_event_stream_failed',
        error: error,
        stackTrace: stackTrace,
      ),
      // Keep listening across errors: the source bloc reconnects on
      // its own (same contract as [AppEventsBloc]).
      cancelOnError: false,
    );
  }

  final FfiClient _client;
  final UrlOpener _urlOpener;

  late final StreamSubscription<AppEventsState> _subscription;

  Future<void> _onLoadRequested(
    AccountsLoadRequested event,
    Emitter<AccountsState> emit,
  ) async {
    try {
      final accounts = await _client.listAccounts();
      if (emit.isDone) return;
      emit(
        state.copyWith(
          accounts: accounts,
          loading: false,
          clearLoadFailure: true,
        ),
      );
    } on AppFailure catch (failure) {
      if (emit.isDone) return;
      emit(state.copyWith(loading: false, loadFailure: failure.message));
    } catch (error, stackTrace) {
      logError('accounts_list_failed', error: error, stackTrace: stackTrace);
      if (emit.isDone) return;
      emit(state.copyWith(loading: false, loadFailure: _genericLoadFailure));
    }
  }

  Future<void> _onEventReceived(
    AccountsEventReceived event,
    Emitter<AccountsState> emit,
  ) async {
    switch (event.event) {
      case AppEvent_AccountStatus(account: final account, state: final status):
        // Parked grants raise the card; every other state (backoff,
        // syncing, stopped, …) lowers it again.
        final ids = Set<int>.of(state.reauthRequired);
        status == reauthRequiredState ? ids.add(account) : ids.remove(account);
        if (!_sameIds(ids, state.reauthRequired)) {
          emit(state.copyWith(reauthRequired: ids));
        }
      case AppEvent_OauthCompleted():
        // A new account exists; refresh the list.
        await _onLoadRequested(const AccountsLoadRequested(), emit);
      case AppEvent_OauthReconnected(account: final account):
        emit(
          state.copyWith(
            clearReconnectPending: true,
            clearReconnectFailure: true,
            reauthRequired: Set<int>.of(state.reauthRequired)..remove(account),
          ),
        );
        await _onLoadRequested(const AccountsLoadRequested(), emit);
      case AppEvent_OauthFailed(reason: final reason):
        // The daemon reports wizard failures too; only a reconnect this
        // shell started surfaces the reason on the card.
        final pending = state.reconnectPending;
        if (pending != null) {
          emit(
            state.copyWith(
              clearReconnectPending: true,
              reconnectFailure: (account: pending, reason: reason),
            ),
          );
        }
      case AppEvent_SyncProgress():
      case AppEvent_OauthUrlReady():
      case AppEvent_Other():
        // No account-list impact.
        break;
    }
  }

  Future<void> _onReconnectRequested(
    AccountReconnectRequested event,
    Emitter<AccountsState> emit,
  ) async {
    if (state.reconnectPending != null) return;
    emit(
      state.copyWith(
        reconnectPending: event.accountId,
        clearReconnectFailure: true,
      ),
    );
    try {
      final url = await _client.startReauth(accountId: event.accountId);
      // The daemon finishes the flow in the background; the outcome
      // arrives as `oauth_reconnected` / `oauth_failed` (F1.6).
      final opened = await _urlOpener.open(url);
      if (emit.isDone) return;
      if (!opened) {
        emit(
          state.copyWith(
            clearReconnectPending: true,
            reconnectFailure: (
              account: event.accountId,
              reason: _browserOpenFailure,
            ),
          ),
        );
      }
    } on AppFailure catch (failure) {
      if (emit.isDone) return;
      emit(
        state.copyWith(
          clearReconnectPending: true,
          reconnectFailure: (account: event.accountId, reason: failure.message),
        ),
      );
    } catch (error, stackTrace) {
      logError(
        'account_reconnect_failed',
        error: error,
        stackTrace: stackTrace,
      );
      if (emit.isDone) return;
      emit(
        state.copyWith(
          clearReconnectPending: true,
          reconnectFailure: (
            account: event.accountId,
            reason: _genericReconnectFailure,
          ),
        ),
      );
    }
  }

  @override
  Future<void> close() async {
    await _subscription.cancel();
    await super.close();
  }
}

/// `account_status` state name published when a refresh-token grant
/// died and the actor parked (SyncState::ReauthRequired).
const reauthRequiredState = 'reauth_required';

const _genericLoadFailure = 'Could not load accounts';
const _genericReconnectFailure = 'Could not start reconnect — try again';
const _browserOpenFailure = 'Could not open the browser';

/// Whether two id sets hold the same members.
bool _sameIds(Set<int> a, Set<int> b) =>
    a.length == b.length && a.containsAll(b);

/// Inputs to [AccountsBloc].
sealed class AccountsEvent {
  const AccountsEvent();
}

/// Re-query the daemon's account list.
final class AccountsLoadRequested extends AccountsEvent {
  const AccountsLoadRequested();
}

/// A daemon event forwarded from the app-wide events bloc.
final class AccountsEventReceived extends AccountsEvent {
  const AccountsEventReceived(this.event);

  /// The forwarded daemon event.
  final AppEvent event;
}

/// Start an in-place reconnect (F1.6) for [accountId].
final class AccountReconnectRequested extends AccountsEvent {
  const AccountReconnectRequested(this.accountId);

  /// Row id of the account whose grant expired.
  final int accountId;
}

/// Snapshot of the accounts surface.
final class AccountsState {
  const AccountsState({
    this.accounts = const [],
    this.loading = true,
    this.loadFailure,
    this.reauthRequired = const {},
    this.reconnectPending,
    this.reconnectFailure,
  });

  /// Newest [FfiClient.listAccounts] result; empty until loaded.
  final List<AccountSummary> accounts;

  /// Whether the first [AccountsLoadRequested] has not answered yet.
  final bool loading;

  /// Human-readable breadcrumb when listing failed; null otherwise.
  final String? loadFailure;

  /// Account ids whose actor reported `reauth_required`.
  final Set<int> reauthRequired;

  /// Account id with a reconnect flow in flight, if any; the daemon
  /// allows a single pending OAuth session, so at most one is set.
  final int? reconnectPending;

  /// Why the last reconnect failed, and on which account's card the
  /// reason belongs; null while a flow is pending or after a clean
  /// outcome.
  final ({int account, String reason})? reconnectFailure;

  /// Copy with selective overrides.
  AccountsState copyWith({
    List<AccountSummary>? accounts,
    bool? loading,
    String? loadFailure,
    bool clearLoadFailure = false,
    Set<int>? reauthRequired,
    int? reconnectPending,
    bool clearReconnectPending = false,
    ({int account, String reason})? reconnectFailure,
    bool clearReconnectFailure = false,
  }) => AccountsState(
    accounts: accounts ?? this.accounts,
    loading: loading ?? this.loading,
    loadFailure: clearLoadFailure ? null : loadFailure ?? this.loadFailure,
    reauthRequired: reauthRequired ?? this.reauthRequired,
    reconnectPending: clearReconnectPending
        ? null
        : reconnectPending ?? this.reconnectPending,
    reconnectFailure: clearReconnectFailure
        ? null
        : reconnectFailure ?? this.reconnectFailure,
  );

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is AccountsState &&
          runtimeType == other.runtimeType &&
          _sameList(other.accounts) &&
          loading == other.loading &&
          loadFailure == other.loadFailure &&
          _sameIds(other.reauthRequired, reauthRequired) &&
          reconnectPending == other.reconnectPending &&
          reconnectFailure == other.reconnectFailure;

  bool _sameList(List<AccountSummary> other) {
    if (accounts.length != other.length) return false;
    for (var i = 0; i < accounts.length; i++) {
      if (accounts[i] != other[i]) return false;
    }
    return true;
  }

  @override
  int get hashCode =>
      Object.hashAll(accounts) ^
      loading.hashCode ^
      loadFailure.hashCode ^
      Object.hashAll(reauthRequired) ^
      reconnectPending.hashCode ^
      reconnectFailure.hashCode;
}
