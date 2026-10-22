import 'dart:async';

import 'package:flutter_bloc/flutter_bloc.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/logging/log.dart';

/// Merges the daemon push stream into invalidation signals for the whole
/// app (ARCHITECTURE §4).
///
/// The bloc listens once to its event stream and re-emits every
/// [AppEvent] as the newest live state; feature BLoCs subscribe
/// selectively to [Bloc.stream] and filter for the events they care
/// about, so unrelated events never trigger work (PERFORMANCE_GUIDELINES
/// §4.5).
class AppEventsBloc extends Bloc<AppEvent, AppEventsState> {
  /// Subscribes to [events]; the subscription lives until [close].
  AppEventsBloc(Stream<AppEvent> events) : super(const AppEventsIdle()) {
    on<AppEvent>(_onAppEvent);
    _subscription = events.listen(
      add,
      onError: (Object error, StackTrace stackTrace) => logError(
        'app_event_stream_failed',
        error: error,
        stackTrace: stackTrace,
      ),
      // Keep listening across errors: the Rust loop reconnects on its own.
      cancelOnError: false,
    );
  }

  late final StreamSubscription<AppEvent> _subscription;

  void _onAppEvent(AppEvent event, Emitter<AppEventsState> emit) {
    emit(AppEventsLive(event));
  }

  @override
  Future<void> close() async {
    await _subscription.cancel();
    await super.close();
  }
}

/// Liveness of the daemon push stream from the GUI's point of view.
sealed class AppEventsState {
  const AppEventsState();
}

/// No event has arrived yet.
final class AppEventsIdle extends AppEventsState {
  const AppEventsIdle();

  @override
  bool operator ==(Object other) => other is AppEventsIdle;

  @override
  int get hashCode => runtimeType.hashCode;
}

/// The newest daemon event; the stream is live.
final class AppEventsLive extends AppEventsState {
  const AppEventsLive(this.event);

  /// The latest event from the daemon.
  final AppEvent event;

  @override
  bool operator ==(Object other) =>
      other is AppEventsLive && other.event == event;

  @override
  int get hashCode => event.hashCode;
}
