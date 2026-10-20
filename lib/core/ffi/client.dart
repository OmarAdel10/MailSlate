import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/src/rust/api/system.dart' as rust;
import 'package:mailslate/src/rust/frb_generated.dart';

export 'package:mailslate/src/rust/api/system.dart'
    show AppEvent, DaemonState, DaemonStatus, PingInfo;

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

  Future<T> _guard<T>(Future<T> Function() call) async {
    try {
      return await call();
    } on rust.DaemonError catch (error) {
      throw mapBridgeError(error);
    }
  }
}
