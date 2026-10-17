import 'dart:async';

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

    final expectation = expectLater(
      bloc.stream,
      emitsInOrder([
        isA<DaemonConnecting>(),
        isA<DaemonConnected>()
            .having((state) => state.daemonVersion, 'daemonVersion', '0.1.0')
            .having((state) => state.protocolVersion, 'protocolVersion', 1),
        emitsDone,
      ]),
    );
    bloc.add(const DaemonStatusRequested());
    await bloc.close();
    await expectation;

    expect(client.startCalls, 0);
  });

  test('spawns the daemon when the status check finds it down', () async {
    final client = FakeFfiClient(
      status: const DaemonStatus(state: DaemonState.unavailable),
      pingResult: const PingInfo(protocolVersion: 1, daemonVersion: '0.1.0'),
    );
    final bloc = DaemonStatusBloc(client);

    final expectation = expectLater(
      bloc.stream,
      emitsInOrder([
        isA<DaemonConnecting>(),
        isA<DaemonConnected>()
            .having((state) => state.daemonVersion, 'daemonVersion', '0.1.0')
            .having((state) => state.protocolVersion, 'protocolVersion', 1),
        emitsDone,
      ]),
    );
    bloc.add(const DaemonStatusRequested());
    await bloc.close();
    await expectation;

    expect(client.startCalls, 1);
  });

  test('reports unavailable with the failure when the spawn fails', () async {
    final client = FakeFfiClient(
      startFailure: const DaemonUnavailableFailure(),
    );
    final bloc = DaemonStatusBloc(client);

    final expectation = expectLater(
      bloc.stream,
      emitsInOrder([
        isA<DaemonConnecting>(),
        isA<DaemonUnavailable>().having(
          (state) => state.failure,
          'failure',
          isA<DaemonUnavailableFailure>(),
        ),
        emitsDone,
      ]),
    );
    bloc.add(const DaemonStatusRequested());
    await bloc.close();
    await expectation;
  });

  test('maps unexpected errors to unavailable without detail', () async {
    final client = FakeFfiClient(statusThrow: StateError('kaboom'));
    final bloc = DaemonStatusBloc(client);

    final expectation = expectLater(
      bloc.stream,
      emitsInOrder([
        isA<DaemonConnecting>(),
        isA<DaemonUnavailable>().having(
          (state) => state.failure,
          'failure',
          isNull,
        ),
        emitsDone,
      ]),
    );
    bloc.add(const DaemonStatusRequested());
    await bloc.close();
    await expectation;
  });

  test('states compare by value', () {
    expect(const DaemonConnecting(), const DaemonConnecting());
    expect(
      const DaemonConnecting().hashCode,
      const DaemonConnecting().hashCode,
    );
    expect(
      const DaemonConnected(daemonVersion: '1.2.3', protocolVersion: 1),
      const DaemonConnected(daemonVersion: '1.2.3', protocolVersion: 1),
    );
    expect(
      const DaemonConnected(
        daemonVersion: '1.2.3',
        protocolVersion: 1,
      ).hashCode,
      const DaemonConnected(
        daemonVersion: '1.2.3',
        protocolVersion: 1,
      ).hashCode,
    );
    expect(const DaemonUnavailable(), const DaemonUnavailable());
    expect(
      const DaemonUnavailable().hashCode,
      const DaemonUnavailable().hashCode,
    );
    expect(
      const DaemonConnected(daemonVersion: '1.2.3', protocolVersion: 1),
      isNot(const DaemonConnected(daemonVersion: '9.9.9', protocolVersion: 1)),
    );
    expect(
      const DaemonUnavailable(failure: StorageFailure()),
      isNot(const DaemonUnavailable()),
    );
  });

  test('drops the result when the bloc closes mid-flight', () async {
    final gate = Completer<void>();
    final client = FakeFfiClient()..statusGate = gate;
    final bloc = DaemonStatusBloc(client);

    final expectation = expectLater(
      bloc.stream,
      emitsInOrder([isA<DaemonConnecting>(), emitsDone]),
    );
    bloc.add(const DaemonStatusRequested());
    await Future<void>.delayed(Duration.zero);

    final closed = bloc.close();
    gate.complete();
    await closed;
    await expectation;

    expect(client.statusCalls, 1);
  });
}
