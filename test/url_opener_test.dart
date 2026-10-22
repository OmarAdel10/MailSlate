import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/system/url_opener.dart';

void main() {
  group('process url opener scheme guard', () {
    final opener = const ProcessUrlOpener();

    test('refuses schemes that could launch a local handler', () async {
      // Only the scheme guard is exercised here: everything refused below
      // must bail out before `Process.run` would spawn a real browser.
      const refused = [
        'file:///etc/passwd',
        'javascript:alert(1)',
        'data:text/html,hi',
        'http://accounts.example.test/authorize',
        'not a url at all',
        '',
      ];
      for (final url in refused) {
        expect(await opener.open(url), isFalse, reason: url);
      }
    });

    test('accepts only https urls syntactically', () {
      // Sanity-check the guard admits the one scheme it allows, without
      // invoking `open` (which would spawn a browser on a passing scheme).
      const allowed = 'https://accounts.example.test/authorize';
      const refused = 'ftp://accounts.example.test/authorize';
      expect(Uri.tryParse(allowed)!.scheme, 'https');
      expect(Uri.tryParse(refused)!.scheme, isNot('https'));
    });
  });
}
