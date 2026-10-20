import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/src/rust/api/system.dart';
import 'package:mailslate/src/rust/frb_generated.dart';

/// Fake of the generated [RustLibApi]: answers without the native library.
///
/// [RustLib.initMock] may run only once per process, so a single instance is
/// created in `setUpAll` and reconfigured per test via [reset].
class FakeRustLibApi extends RustLibApi {
  DaemonStatus? status;
  PingInfo? pingResult;
  DaemonError? error;

  int pingCalls = 0;

  /// Stream the fake [FfiClient.events] can subscribe to in tests.
  Stream<AppEvent> eventStream = const Stream<AppEvent>.empty();

  void reset() {
    status = null;
    pingResult = null;
    error = null;
    pingCalls = 0;
    eventStream = const Stream<AppEvent>.empty();
  }

  @override
  Stream<AppEvent> crateApiSystemAppEvents() => eventStream;

  @override
  Future<DaemonStatus> crateApiSystemDaemonStatus() async {
    final value = status;
    if (value == null) {
      throw StateError('no status configured');
    }
    return value;
  }

  @override
  Future<void> crateApiSystemInitApp() async {}

  @override
  Future<String> crateApiSystemDaemonLogDir() async => '';

  @override
  Future<PingInfo> crateApiSystemPing() async {
    pingCalls += 1;
    final failure = error;
    if (failure != null) {
      throw failure;
    }
    return pingResult ??
        const PingInfo(protocolVersion: 1, daemonVersion: '0.0.0-test');
  }

  @override
  Future<PingInfo> crateApiSystemStartDaemon() async {
    return pingResult ??
        const PingInfo(protocolVersion: 1, daemonVersion: '0.0.0-test');
  }
}

void main() {
  final api = FakeRustLibApi();

  setUpAll(() {
    RustLib.initMock(api: api);
  });

  setUp(api.reset);

  test('ping surfaces a DaemonError as an AppFailure via the guard', () async {
    api.error = DaemonError(
      kind: 'daemon_unavailable',
      message: 'could not connect to /tmp/daemon.sock',
      retryable: true,
    );

    await expectLater(
      const FfiClient().ping(),
      throwsA(
        isA<DaemonUnavailableFailure>()
            .having(
              (failure) => failure.message,
              'message',
              'could not connect to /tmp/daemon.sock',
            )
            .having((failure) => failure.retryable, 'retryable', isTrue),
      ),
    );
    expect(api.pingCalls, 1);
  });

  test('daemonStatus and startDaemon pass values through', () async {
    api.status = const DaemonStatus(
      state: DaemonState.running,
      daemonVersion: '9.9.9',
      protocolVersion: 1,
    );
    api.pingResult = const PingInfo(protocolVersion: 1, daemonVersion: '9.9.9');

    const client = FfiClient();
    final status = await client.daemonStatus();
    expect(status.state, DaemonState.running);
    expect(status.daemonVersion, '9.9.9');
    expect(status.protocolVersion, 1);

    final info = await client.startDaemon();
    expect(info.daemonVersion, '9.9.9');
    expect(info.protocolVersion, 1);
  });

  test('events stays open and empty', () async {
    final events = await const FfiClient().events().toList();
    expect(events, isEmpty);
  });
}
