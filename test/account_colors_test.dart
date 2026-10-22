import 'dart:ui' show Color;

import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/theme/tokens.dart';
import 'package:mailslate/features/accounts/account_colors.dart';
import 'package:mailslate/src/rust/api/accounts.dart';

AccountSummary _account({int id = 1, String? color}) => AccountSummary(
  id: id,
  displayName: 'Work',
  email: 'work@example.com',
  provider: 'gmail',
  authType: 'oauth2',
  syncWindowDays: 365,
  color: color,
);

void main() {
  final tokens = AppTokens.light;

  group('parseAccountHex', () {
    test('parses a #RRGGBB string', () {
      final color = parseAccountHex('#EC4899');

      expect(color, const Color(0xFFEC4899));
    });

    test('rejects malformed values', () {
      expect(parseAccountHex('nope'), isNull);
      expect(parseAccountHex('#GGGGGG'), isNull);
      expect(parseAccountHex('#12345'), isNull);
      expect(parseAccountHex('EC4899'), isNull);
      expect(parseAccountHex(null), isNull);
    });
  });

  group('accountDotColor', () {
    test('prefers the stored account color', () {
      final color = accountDotColor(_account(color: '#FB923C'), tokens);

      expect(color, tokens.badgeOrange);
    });

    test('falls back to a deterministic palette color by id', () {
      final palette = [
        tokens.badgeOrange,
        tokens.badgePink,
        tokens.badgeViolet,
        tokens.badgeEmerald,
      ];
      for (var id = 1; id <= 8; id++) {
        final color = accountDotColor(_account(id: id), tokens);
        expect(color, palette[id % palette.length]);
      }
    });
  });
}
