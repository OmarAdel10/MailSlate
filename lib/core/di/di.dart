import 'package:get_it/get_it.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/logging/log_opener.dart';
import 'package:mailslate/core/system/url_opener.dart';

/// The app-wide service locator.
final GetIt getIt = GetIt.instance;

/// Registers production dependencies.
///
/// Pass [client], [logOpener] or [urlOpener] to substitute fakes in tests;
/// the defaults talk to the Rust bridge, the desktop's file handler, and
/// the desktop's browser.
void configureDependencies({
  FfiClient? client,
  LogOpener? logOpener,
  UrlOpener? urlOpener,
}) {
  getIt.registerSingleton<FfiClient>(client ?? const FfiClient());
  getIt.registerSingleton<LogOpener>(logOpener ?? const XdgOpenLogOpener());
  getIt.registerSingleton<UrlOpener>(urlOpener ?? const ProcessUrlOpener());
}
