import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/src/rust/api/system.dart' as rust;
import 'package:mailslate/src/rust/frb_generated.dart';

export 'package:mailslate/src/rust/api/system.dart'
    show DaemonState, DaemonStatus, PingInfo;

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

  /// Daemon push events (new mail, sync progress, …).
  ///
  /// The daemon event channel ships with the M2 sync engine; until then the
  /// stream is empty and stays open.
  Stream<Object?> events() => const Stream<Object?>.empty();

  Future<T> _guard<T>(Future<T> Function() call) async {
    try {
      return await call();
    } on rust.DaemonError catch (error) {
      throw mapBridgeError(error);
    }
  }
}
