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
/// still be missing on early boot); the failure is logged and the next
/// listener dials again instead of crashing the one that arrived first.
///
/// `Stream.asBroadcastStream` is deliberately not used: it pins the
/// source subscription for the stream's whole lifetime and never
/// re-listens, which would keep a heartbeat connection open with no
/// consumer.
Stream<AppEvent> appEventStream(FfiClient client) {
  late final StreamController<AppEvent> controller;
  StreamSubscription<AppEvent>? source;

  controller = StreamController<AppEvent>.broadcast(
    onListen: () {
      if (source != null) return;
      try {
        source = client.events().listen(
          controller.add,
          onError: controller.addError,
          onDone: controller.close,
        );
      } catch (error, stackTrace) {
        // Leave `source` null so the next listener dials again.
        logError('app_event_dial_failed', error: error, stackTrace: stackTrace);
      }
    },
    onCancel: () async {
      final subscription = source;
      source = null;
      await subscription?.cancel();
    },
  );

  return controller.stream;
}
