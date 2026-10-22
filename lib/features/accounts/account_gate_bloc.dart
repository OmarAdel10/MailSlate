import 'package:flutter_bloc/flutter_bloc.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/core/logging/log.dart';

/// Decides between the first-run wizard and the shell at startup
/// (USER_FLOW §3): accounts present → shell, zero accounts → wizard.
class AccountGateBloc extends Bloc<AccountGateEvent, AccountGateState> {
  AccountGateBloc(this._client) : super(const AccountGateChecking()) {
    on<AccountGateCheckRequested>(_onCheckRequested);
  }

  final FfiClient _client;

  Future<void> _onCheckRequested(
    AccountGateCheckRequested event,
    Emitter<AccountGateState> emit,
  ) async {
    emit(const AccountGateChecking());
    try {
      final accounts = await _client.listAccounts();
      if (emit.isDone) return;
      if (accounts.isEmpty) {
        emit(const AccountGateNeedsAccount());
        return;
      }
      emit(AccountGateReady(accounts));
    } on AppFailure catch (failure) {
      if (emit.isDone) return;
      logWarning('account_gate_unavailable', error: failure);
      emit(AccountGateFailure(failure.message));
    } catch (error, stackTrace) {
      if (emit.isDone) return;
      logError(
        'account_gate_check_failed',
        error: error,
        stackTrace: stackTrace,
      );
      emit(const AccountGateFailure('Something went wrong'));
    }
  }
}

/// Requests an account listing (startup, after the wizard finishes).
sealed class AccountGateEvent {
  const AccountGateEvent();
}

/// Lists accounts and picks the shell or the wizard.
final class AccountGateCheckRequested extends AccountGateEvent {
  const AccountGateCheckRequested();
}

/// Startup gate state: which surface the app should show.
sealed class AccountGateState {
  const AccountGateState();
}

/// A listing is in flight.
final class AccountGateChecking extends AccountGateState {
  const AccountGateChecking();

  @override
  bool operator ==(Object other) => other is AccountGateChecking;

  @override
  int get hashCode => runtimeType.hashCode;
}

/// The listing succeeded and found no accounts: run the wizard.
final class AccountGateNeedsAccount extends AccountGateState {
  const AccountGateNeedsAccount();

  @override
  bool operator ==(Object other) => other is AccountGateNeedsAccount;

  @override
  int get hashCode => runtimeType.hashCode;
}

/// The listing succeeded; the shell can render around these accounts.
final class AccountGateReady extends AccountGateState {
  const AccountGateReady(this.accounts);

  /// The accounts the daemon reported.
  final List<AccountSummary> accounts;

  @override
  bool operator ==(Object other) =>
      other is AccountGateReady && other.accounts == accounts;

  @override
  int get hashCode => Object.hashAll([runtimeType, ...accounts]);
}

/// The listing failed; the shell still renders and its daemon banner
/// reports the outage (M1 resilience).
final class AccountGateFailure extends AccountGateState {
  const AccountGateFailure(this.message);

  /// Why the listing failed, when known.
  final String message;

  @override
  bool operator ==(Object other) =>
      other is AccountGateFailure && other.message == message;

  @override
  int get hashCode => Object.hash(runtimeType, message);
}
