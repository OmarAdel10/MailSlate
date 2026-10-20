import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/features/shell/app_events_bloc.dart';

void main() {
  test('initial state is idle', () {
    final controller = StreamController<AppEvent>();
    final bloc = AppEventsBloc(controller.stream);

    expect(bloc.state, const AppEventsIdle());

    bloc.close();
    controller.close();
  });

  test('forwards every event as the newest live state', () async {
    final controller = StreamController<AppEvent>();
    final bloc = AppEventsBloc(controller.stream);

    final expectation = expectLater(
      bloc.stream,
      emitsInOrder([
        isA<AppEventsLive>().having(
          (state) => state.event,
          'event',
          const AppEvent.accountStatus(account: 1, state: 'syncing'),
        ),
        isA<AppEventsLive>().having(
          (state) => state.event,
          'event',
          const AppEvent.oauthCompleted(email: 'a@b.c'),
        ),
        emitsDone,
      ]),
    );

    controller.add(const AppEvent.accountStatus(account: 1, state: 'syncing'));
    controller.add(const AppEvent.oauthCompleted(email: 'a@b.c'));
    await Future<void>.delayed(Duration.zero);

    await controller.close();
    await bloc.close();
    await expectation;
  });

  test('close cancels the source subscription', () async {
    var sourceCancelled = false;
    final controller = StreamController<AppEvent>(
      onCancel: () => sourceCancelled = true,
    );
    final bloc = AppEventsBloc(controller.stream);
    await Future<void>.delayed(Duration.zero);

    await bloc.close();
    await Future<void>.delayed(Duration.zero);

    expect(sourceCancelled, isTrue);
  });

  test('states compare by value', () {
    const event = AppEvent.accountStatus(account: 1, state: 'syncing');
    expect(const AppEventsIdle(), const AppEventsIdle());
    expect(const AppEventsIdle().hashCode, const AppEventsIdle().hashCode);
    expect(AppEventsLive(event), AppEventsLive(event));
    expect(AppEventsLive(event).hashCode, AppEventsLive(event).hashCode);
    expect(
      AppEventsLive(event),
      isNot(const AppEventsLive(AppEvent.other(name: 'different'))),
    );
  });
}
