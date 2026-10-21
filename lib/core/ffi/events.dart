import 'dart:async';

import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/logging/log.dart';

/// Turns the single-subscription FRB event stream into an app-lifetime
/// broadcast stream of typed [AppEvent]s (ARCHITECTURE §4:
/// `core/ffi/events.dart`).
///
/// The daemon event connection lives only while at least one listener is
/// subscribed: the first listener dials the connection up, cancelling the
/// last listener tears it down (heartbeats stop, the daemon drops the
/// idle connection), and a later listener dials a fresh one. Events
/// published while nobody listens are dropped — feature BLoCs re-query
/// their data when they start listening instead of replaying history.
///
/// A dial can fail before a subscription exists (the native bridge may
/// still be missing on early boot). The failure is logged and retried
/// with exponential backoff while listeners are waiting, so the first
/// listener recovers on its own instead of starving for the daemon's
/// whole lifetime.
///
/// `Stream.asBroadcastStream` is deliberately not used: it pins the
/// source subscription for the stream's whole lifetime and never
/// re-listens, which would keep a heartbeat connection open with no
/// consumer.
///
/// [dialRetryDelay] is the first backoff step; each further failure
/// doubles it, capped at 32×.
Stream<AppEvent> appEventStream(
  FfiClient client, {
  Duration dialRetryDelay = const Duration(milliseconds: 500),
}) {
  late final StreamController<AppEvent> controller;
  StreamSubscription<AppEvent>? source;
  Timer? retry;
  var listening = false;
  var failures = 0;

  void dial() {
    retry = null;
    if (source != null || !listening) return;
    try {
      source = client.events().listen(
        controller.add,
        onError: controller.addError,
        onDone: controller.close,
      );
      failures = 0;
    } catch (error, stackTrace) {
      // Leave `source` null so the retry can dial again.
      failures += 1;
      logError('app_event_dial_failed', error: error, stackTrace: stackTrace);
      final exponent = failures > 6 ? 6 : failures - 1;
      retry ??= Timer(dialRetryDelay * (1 << exponent), dial);
    }
  }

  controller = StreamController<AppEvent>.broadcast(
    onListen: () {
      listening = true;
      dial();
    },
    onCancel: () async {
      listening = false;
      retry?.cancel();
      retry = null;
      failures = 0;
      final subscription = source;
      source = null;
      await subscription?.cancel();
    },
  );

  return controller.stream;
}
