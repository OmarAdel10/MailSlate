import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/failures/failures.dart';

/// Test double for [FfiClient]: never loads Rust, never touches a daemon.
class FakeFfiClient extends FfiClient {
  FakeFfiClient({
    this.status = const DaemonStatus(state: DaemonState.unavailable),
    this.pingResult = const PingInfo(
      protocolVersion: 1,
      daemonVersion: '0.0.0-test',
    ),
    this.startFailure,
    this.statusFailure,
  });

  /// Returned by [daemonStatus] when [statusFailure] is null.
  DaemonStatus status;

  /// Returned by [startDaemon] and [ping] when no failure is set.
  PingInfo pingResult;

  /// When set, [startDaemon] throws it.
  AppFailure? startFailure;

  /// When set, [daemonStatus] throws it.
  AppFailure? statusFailure;

  /// How often [startDaemon] was called.
  int startCalls = 0;

  @override
  Future<void> initApp() async {}

  @override
  Future<DaemonStatus> daemonStatus() async {
    final failure = statusFailure;
    if (failure != null) {
      throw failure;
    }
    return status;
  }

  @override
  Future<PingInfo> startDaemon() async {
    startCalls += 1;
    final failure = startFailure;
    if (failure != null) {
      throw failure;
    }
    return pingResult;
  }

  @override
  Future<PingInfo> ping() async => pingResult;
}
