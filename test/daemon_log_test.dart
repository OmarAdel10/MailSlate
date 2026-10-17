import 'dart:io';

import 'package:mailslate/core/logging/daemon_log.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  late Directory dir;

  setUp(() async {
    dir = await Directory.systemTemp.createTemp('mailslate-log-');
  });

  tearDown(() async {
    await dir.delete(recursive: true);
  });

  test('returns null when the directory has no daemon log', () {
    expect(newestDaemonLog(dir.path), isNull);
  });

  test('returns null when files do not match the rotation pattern', () async {
    await File('${dir.path}/notes.txt').writeAsString('x');
    await File('${dir.path}/daemon.txt').writeAsString('x');

    expect(newestDaemonLog(dir.path), isNull);
  });

  test('picks the newest rotation file', () async {
    final older = File('${dir.path}/daemon.2026-10-15.log');
    final newer = File('${dir.path}/daemon.2026-10-16.log');
    await older.writeAsString('older');
    await Future<void>.delayed(const Duration(milliseconds: 20));
    await newer.writeAsString('newer');

    expect(newestDaemonLog(dir.path), newer.path);
  });

  test('tolerates a missing directory', () {
    expect(newestDaemonLog('${dir.path}/nope'), isNull);
  });
}
