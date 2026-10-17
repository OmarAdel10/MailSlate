import 'dart:io';

/// Opens a file with the desktop's default handler.
///
/// Injected via DI so widget tests can record calls without spawning
/// processes.
abstract class LogOpener {
  /// Opens [path]; false when the handler is missing or fails.
  Future<bool> open(String path);
}

/// Linux implementation: `xdg-open`.
class XdgOpenLogOpener implements LogOpener {
  const XdgOpenLogOpener();

  @override
  Future<bool> open(String path) async {
    if (!Platform.isLinux) {
      return false;
    }
    try {
      final result = await Process.run('xdg-open', [path]);
      return result.exitCode == 0;
    } on ProcessException {
      return false;
    }
  }
}
