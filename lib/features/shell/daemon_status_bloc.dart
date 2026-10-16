import 'package:flutter_bloc/flutter_bloc.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/core/logging/log.dart';

/// Watches daemon liveness for the toolbar cluster and the paused banner.
class DaemonStatusBloc extends Bloc<DaemonStatusEvent, DaemonStatusState> {
  DaemonStatusBloc(this._client) : super(const DaemonConnecting()) {
    on<DaemonStatusRequested>(_onRequested);
  }

  final FfiClient _client;

  Future<void> _onRequested(
    DaemonStatusRequested event,
    Emitter<DaemonStatusState> emit,
  ) async {
    emit(const DaemonConnecting());
    try {
      final status = await _client.daemonStatus();
      if (status.state == DaemonState.running) {
        emit(
          DaemonConnected(
            daemonVersion: status.daemonVersion,
            protocolVersion: status.protocolVersion,
          ),
        );
        return;
      }
      // The daemon is down: spawn it, as the GUI owns its lifecycle
      // (USER_FLOW §9).
      final ping = await _client.startDaemon();
      emit(
        DaemonConnected(
          daemonVersion: ping.daemonVersion,
          protocolVersion: ping.protocolVersion,
        ),
      );
    } on AppFailure catch (failure) {
      logWarning('daemon_unavailable', error: failure);
      emit(DaemonUnavailable(failure: failure));
    } catch (error, stackTrace) {
      logError('daemon_status_failed', error: error, stackTrace: stackTrace);
      emit(const DaemonUnavailable());
    }
  }
}

/// Requests a daemon liveness check, spawning the daemon when it is down.
sealed class DaemonStatusEvent {
  const DaemonStatusEvent();
}

/// Checks daemon liveness once (shell startup, retry, refresh).
final class DaemonStatusRequested extends DaemonStatusEvent {
  const DaemonStatusRequested();
}

/// Liveness of the daemon from the GUI's point of view.
sealed class DaemonStatusState {
  const DaemonStatusState();
}

/// A liveness check is in flight.
final class DaemonConnecting extends DaemonStatusState {
  const DaemonConnecting();

  @override
  bool operator ==(Object other) => other is DaemonConnecting;

  @override
  int get hashCode => runtimeType.hashCode;
}

/// The daemon answered; pushes arrive once sync lands in M2.
final class DaemonConnected extends DaemonStatusState {
  const DaemonConnected({this.daemonVersion, this.protocolVersion});

  /// The daemon's crate version, when reported.
  final String? daemonVersion;

  /// The IPC protocol version, when reported.
  final int? protocolVersion;

  @override
  bool operator ==(Object other) =>
      other is DaemonConnected &&
      other.daemonVersion == daemonVersion &&
      other.protocolVersion == protocolVersion;

  @override
  int get hashCode => Object.hash(daemonVersion, protocolVersion);
}

/// The daemon is unreachable; the shell shows the paused banner.
final class DaemonUnavailable extends DaemonStatusState {
  const DaemonUnavailable({this.failure});

  /// Why the daemon is unreachable, when known.
  final AppFailure? failure;

  @override
  bool operator ==(Object other) =>
      other is DaemonUnavailable && other.failure == failure;

  @override
  int get hashCode => failure.hashCode;
}
