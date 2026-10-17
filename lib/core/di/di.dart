import 'package:get_it/get_it.dart';
import 'package:mailslate/core/ffi/client.dart';

/// The app-wide service locator.
final GetIt getIt = GetIt.instance;

/// Registers production dependencies.
///
/// Pass [client] to substitute a fake in tests; the default [FfiClient] talks
/// to the Rust bridge.
void configureDependencies({FfiClient? client}) {
  getIt.registerSingleton<FfiClient>(client ?? const FfiClient());
}
