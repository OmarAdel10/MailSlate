import 'package:get_it/get_it.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/logging/log_opener.dart';

/// The app-wide service locator.
final GetIt getIt = GetIt.instance;

/// Registers production dependencies.
///
/// Pass [client] or [logOpener] to substitute fakes in tests; the defaults
/// talk to the Rust bridge and the desktop's file handler.
void configureDependencies({FfiClient? client, LogOpener? logOpener}) {
  getIt.registerSingleton<FfiClient>(client ?? const FfiClient());
  getIt.registerSingleton<LogOpener>(logOpener ?? const XdgOpenLogOpener());
}
