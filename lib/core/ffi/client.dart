import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/src/rust/api/accounts.dart' as rust_accounts;
import 'package:mailslate/src/rust/api/system.dart' as rust;
import 'package:mailslate/src/rust/frb_generated.dart';

export 'package:mailslate/src/rust/api/accounts.dart'
    show
        AccountPatch,
        AccountSummary,
        ConnectionStep,
        ConnectionTest,
        ImapDraft;
export 'package:mailslate/src/rust/api/system.dart'
    show
        AppEvent,
        AppEvent_AccountStatus,
        AppEvent_OauthCompleted,
        AppEvent_OauthFailed,
        AppEvent_OauthReconnected,
        AppEvent_OauthUrlReady,
        AppEvent_Other,
        AppEvent_SyncProgress,
        DaemonState,
        DaemonStatus,
        PingInfo;

/// Thin façade over the generated Rust bindings.
///
/// The UI depends on this type, never on the generated bindings directly, so
/// tests can inject a fake without loading the Rust library or starting the
/// daemon. Methods map bridge errors onto [AppFailure].
class FfiClient {
  const FfiClient();

  /// Loads the Rust library and runs the bridge init hook (`init_app`).
  Future<void> initApp() => RustLib.init();

  /// Pings the daemon; never spawns one.
  ///
  /// Throws a [DaemonUnavailableFailure] when the daemon does not answer.
  Future<rust.PingInfo> ping() => _guard(rust.ping);

  /// Reports daemon liveness and versions.
  ///
  /// Never throws for an unreachable daemon: the status reports
  /// [DaemonState.unavailable] instead.
  Future<rust.DaemonStatus> daemonStatus() => _guard(rust.daemonStatus);

  /// Ensures a daemon is running (spawning it when needed) and pings it.
  Future<rust.PingInfo> startDaemon() => _guard(rust.startDaemon);

  /// The daemon's log directory, as an absolute path.
  ///
  /// Pure path lookup on the Rust side; works while the daemon is down.
  Future<String> daemonLogDir() => _guard(rust.daemonLogDir);

  /// Daemon push events as a single-subscription typed stream.
  ///
  /// Never errors and never ends on its own: the Rust loop heartbeats
  /// past the daemon's idle timeout and reconnects with backoff while
  /// the daemon is down (ARCHITECTURE §5). Cancelling the subscription
  /// tears the event connection down; listening again dials it back up.
  Stream<rust.AppEvent> events() => rust.appEvents();

  /// Lists every account row the daemon knows about.
  ///
  /// Empty list when no account has been added yet.
  Future<List<rust_accounts.AccountSummary>> listAccounts() =>
      _guard(rust_accounts.listAccounts);

  /// Adds a password-authenticated IMAP/SMTP account.
  ///
  /// Returns the new account id. Throws a [ValidationFailure] when the
  /// daemon rejects the draft.
  Future<int> addImapAccount({
    required rust_accounts.ImapDraft draft,
    required String password,
  }) => _guard(
    () => rust_accounts.addImapAccount(draft: draft, password: password),
  );

  /// Starts the OAuth flow for [provider] and returns the authorize url.
  ///
  /// The daemon completes the flow in the background; the resulting
  /// account is announced on the [events] stream.
  Future<String> startOauth({required String provider}) =>
      _guard(() => rust_accounts.startOauth(provider: provider));

  /// Starts an in-place reconnect for [accountId] and returns the
  /// authorize url.
  ///
  /// Used when an account's grant expired: the daemon refreshes the
  /// account's own tokens and restarts its sync actor; the outcome is
  /// announced as `oauth_reconnected` on the [events] stream.
  Future<String> startReauth({required int accountId}) =>
      _guard(() => rust_accounts.startReauth(accountId: accountId));

  /// Adopts the OAuth account the daemon just finished authenticating.
  ///
  /// Returns the new account id.
  Future<int> completeOauthAccount({
    required String displayName,
    required String email,
    String? color,
  }) => _guard(
    () => rust_accounts.completeOauthAccount(
      displayName: displayName,
      email: email,
      color: color,
    ),
  );

  /// Patches an account row.
  ///
  /// `AccountPatch.clearColor` wins over a set `AccountPatch.color` so the
  /// tri-state (untouched / cleared / set) crosses the FFI boundary without
  /// `String??` parameters.
  Future<void> updateAccount({
    required int accountId,
    required rust_accounts.AccountPatch patch,
  }) => _guard(
    () => rust_accounts.updateAccount(accountId: accountId, patch: patch),
  );

  /// Removes an account row.
  ///
  /// Local synced data is kept (daemon `delete_data` is not wired yet).
  Future<void> removeAccount({required int accountId}) =>
      _guard(() => rust_accounts.removeAccount(accountId: accountId));

  /// Probes a would-be account before the wizard saves it.
  ///
  /// Both steps always come back, so the wizard can show the
  /// `IMAP ✓ / SMTP ✓` row even when one leg fails (USER_FLOW §3).
  Future<rust_accounts.ConnectionTest> testConnection({
    required rust_accounts.ImapDraft draft,
    required String password,
  }) => _guard(
    () => rust_accounts.testConnection(draft: draft, password: password),
  );

  Future<T> _guard<T>(Future<T> Function() call) async {
    try {
      return await call();
    } on rust.DaemonError catch (error) {
      throw mapBridgeError(error);
    }
  }
}
