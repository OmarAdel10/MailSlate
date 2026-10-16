import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/features/shell/daemon_status_bloc.dart';

import 'support/fake_ffi_client.dart';

void main() {
  test('reports connected from a running daemon status', () async {
    final client = FakeFfiClient(
      status: const DaemonStatus(
        state: DaemonState.running,
        daemonVersion: '0.1.0',
        protocolVersion: 1,
      ),
    );
    final bloc = DaemonStatusBloc(client);
    addTearDown(bloc.close);

    final expectation = expectLater(
      bloc.stream,
      emitsInOrder([
        isA<DaemonConnecting>(),
        isA<DaemonConnected>()
            .having((state) => state.daemonVersion, 'daemonVersion', '0.1.0')
            .having((state) => state.protocolVersion, 'protocolVersion', 1),
      ]),
    );
    bloc.add(const DaemonStatusRequested());
    await expectation;

    expect(client.startCalls, 0);
  });

  test('spawns the daemon when the status check finds it down', () async {
    final client = FakeFfiClient(
      pingResult: const PingInfo(protocolVersion: 1, daemonVersion: '0.1.0'),
    );
    final bloc = DaemonStatusBloc(client);
    addTearDown(bloc.close);

    final expectation = expectLater(
      bloc.stream,
      emitsInOrder([isA<DaemonConnecting>(), isA<DaemonConnected>()]),
    );
    bloc.add(const DaemonStatusRequested());
    await expectation;

    expect(client.startCalls, 1);
  });

  test('reports unavailable with the failure when the spawn fails', () async {
    final client = FakeFfiClient(
      startFailure: const DaemonUnavailableFailure(),
    );
    final bloc = DaemonStatusBloc(client);
    addTearDown(bloc.close);

    final expectation = expectLater(
      bloc.stream,
      emitsInOrder([
        isA<DaemonConnecting>(),
        isA<DaemonUnavailable>().having(
          (state) => state.failure,
          'failure',
          isA<DaemonUnavailableFailure>(),
        ),
      ]),
    );
    bloc.add(const DaemonStatusRequested());
    await expectation;
  });
}
