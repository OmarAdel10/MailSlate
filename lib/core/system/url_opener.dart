import 'dart:io';

import 'package:mailslate/core/logging/log.dart';

/// Opens a URL with the desktop's default browser.
///
/// Injected via DI so bloc and widget tests can record calls without
/// spawning processes (the same seam as `LogOpener`).
abstract class UrlOpener {
  /// Opens [url]; false when no handler is available or the launch fails.
  Future<bool> open(String url);
}

/// Schemes the opener is allowed to launch. Only `https` URLs are opened;
/// anything else (`file:`, `javascript:`, `data:`, …) is refused and logged
/// so an authorize URL can never be used to launch a local handler.
const _allowedUrlSchemes = {'https'};

/// Launches the default browser with a platform-specific command.
class ProcessUrlOpener implements UrlOpener {
  const ProcessUrlOpener();

  @override
  Future<bool> open(String url) async {
    final uri = Uri.tryParse(url);
    if (uri == null || !_allowedUrlSchemes.contains(uri.scheme)) {
      logWarning('url_opener_rejected_scheme', error: url);
      return false;
    }
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
        // rundll32 url.dll,FileProtocolHandler passes the URL as a single
        // argument to ShellExecute. `cmd /c start '' url` would split the
        // authorize URL on '&' (breaking OAuth on Windows) and is
        // injection-prone.
        final result = await Process.run('rundll32', [
          'url.dll,FileProtocolHandler',
          url,
        ]);
        return result.exitCode == 0;
      }
      return false;
    } on ProcessException {
      return false;
    }
  }
}
