import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/ffi/events.dart';

import 'support/fake_ffi_client.dart';

void main() {
  test('fans every event out to all listeners', () async {
    final client = FakeFfiClient()
      ..eventStream = () => Stream<AppEvent>.fromIterable([
        const AppEvent.accountStatus(account: 7, state: 'idle'),
        const AppEvent.syncProgress(
          account: 7,
          mailbox: 'INBOX',
          phase: 'initial',
          done: 1,
          total: 3,
        ),
      ]);
    final broadcast = appEventStream(client);

    final first = broadcast.toList();
    final second = broadcast.toList();
    final results = await Future.wait([first, second]);

    expect(results.first, [
      const AppEvent.accountStatus(account: 7, state: 'idle'),
      const AppEvent.syncProgress(
        account: 7,
        mailbox: 'INBOX',
        phase: 'initial',
        done: 1,
        total: 3,
      ),
    ]);
    expect(results.last, results.first);
  });

  test(
    'cancelling the last listener tears the source connection down',
    () async {
      var sourceCancelled = false;
      final controller = StreamController<AppEvent>(
        onCancel: () => sourceCancelled = true,
      );
      final client = FakeFfiClient()..eventStream = () => controller.stream;
      final broadcast = appEventStream(client);

      final subscription = broadcast.listen(null);
      await pumpEventQueue();

      await subscription.cancel();
      await pumpEventQueue();

      expect(sourceCancelled, isTrue);
    },
  );

  test('a listener that arrives later dials the source back up', () async {
    var subscriptions = 0;
    StreamController<AppEvent>? active;
    Stream<AppEvent> freshSource() {
      active = StreamController<AppEvent>(onListen: () => subscriptions += 1);
      return active!.stream;
    }

    final client = FakeFfiClient()..eventStream = freshSource;
    final broadcast = appEventStream(client);

    final firstSubscription = broadcast.listen(null);
    await pumpEventQueue();
    await firstSubscription.cancel();
    await pumpEventQueue();

    final secondSubscription = broadcast.listen(null);
    await pumpEventQueue();
    await secondSubscription.cancel();
    await pumpEventQueue();

    expect(subscriptions, 2);
    expect(active!.hasListener, isFalse);
  });

  test(
    'a failing source dial does not crash the listener and retries',
    () async {
      var calls = 0;
      final client = FakeFfiClient()
        ..eventStream = () {
          calls += 1;
          if (calls == 1) {
            // The bridge can miss its dylib at first boot; the dial throws
            // before a subscription exists.
            throw StateError('bridge not ready');
          }
          return Stream.value(const AppEvent.other(name: 'late'));
        };
      final broadcast = appEventStream(client);

      // The first dial fails: the broadcast stream must swallow the throw
      // instead of crashing the listener that dialed it.
      final first = broadcast.listen(null);
      await pumpEventQueue();
      await first.cancel();
      await pumpEventQueue();

      final events = await broadcast.toList();

      expect(calls, 2);
      expect(events, const [AppEvent.other(name: 'late')]);
    },
  );

  test('a failed dial retries while a listener is waiting', () async {
    var calls = 0;
    final client = FakeFfiClient()
      ..eventStream = () {
        calls += 1;
        if (calls == 1) {
          throw StateError('bridge not ready');
        }
        return Stream.value(const AppEvent.other(name: 'late'));
      };
    final broadcast = appEventStream(
      client,
      dialRetryDelay: const Duration(milliseconds: 5),
    );

    // The listener that triggered the failed dial must not starve: the
    // stream dials again on its own once the bridge appears.
    final events = await broadcast.toList();

    expect(calls, 2);
    expect(events, const [AppEvent.other(name: 'late')]);
  });
}
