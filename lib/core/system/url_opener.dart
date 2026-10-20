import 'dart:io';

/// Opens a URL with the desktop's default browser.
///
/// Injected via DI so bloc and widget tests can record calls without
/// spawning processes (the same seam as `LogOpener`).
abstract class UrlOpener {
  /// Opens [url]; false when no handler is available or the launch fails.
  Future<bool> open(String url);
}

/// Launches the default browser with a platform-specific command.
class ProcessUrlOpener implements UrlOpener {
  const ProcessUrlOpener();

  @override
  Future<bool> open(String url) async {
    try {
      if (Platform.isLinux) {
        final result = await Process.run('xdg-open', [url]);
        return result.exitCode == 0;
      }
      if (Platform.isMacOS) {
        final result = await Process.run('open', [url]);
        return result.exitCode == 0;
      }
      if (Platform.isWindows) {
        final result = await Process.run('cmd', ['/c', 'start', '', url]);
        return result.exitCode == 0;
      }
      return false;
    } on ProcessException {
      return false;
    }
  }
}
