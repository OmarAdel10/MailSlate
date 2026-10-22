import 'package:flutter/material.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/theme/tokens.dart';

/// Accent-color palette offered to accounts, as `#RRGGBB` strings.
///
/// Mirrors the badge pastel set (DESIGN §5.4: orange, pink, violet,
/// emerald); the hex spellings stay plain so they round-trip through
/// the daemon's free-form `color` column and the keyring-free wire.
const accountPaletteHexes = [
  '#FB923C', // orange
  '#EC4899', // pink
  '#8B5CF6', // violet
  '#34D399', // emerald
];

/// Parses an account's stored `#RRGGBB` accent, or picks a
/// deterministic badge pastel from the account id (USER_FLOW §2:
/// every account shows a color dot even before a color is chosen).
Color accountDotColor(AccountSummary account, AppTokens tokens) {
  final stored = parseAccountHex(account.color);
  if (stored != null) return stored;
  final palette = [
    tokens.badgeOrange,
    tokens.badgePink,
    tokens.badgeViolet,
    tokens.badgeEmerald,
  ];
  return palette[account.id % palette.length];
}

/// Parses `#RRGGBB` into a [Color]; null when [hex] is absent or
/// malformed (a foreign value never breaks rendering).
Color? parseAccountHex(String? hex) {
  if (hex == null || !hex.startsWith('#') || hex.length != 7) return null;
  final value = int.tryParse(hex.substring(1), radix: 16);
  return value == null ? null : Color(0xFF000000 | value);
}
