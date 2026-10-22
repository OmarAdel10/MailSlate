import 'package:flutter/material.dart';
import 'package:flutter_bloc/flutter_bloc.dart';
import 'package:mailslate/core/di/di.dart';
import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/theme/tokens.dart';
import 'package:material_symbols_icons/symbols.dart';

import 'account_colors.dart';
import 'accounts_bloc.dart';

/// Opens the per-account settings sheet (F1.5): display name, accent
/// color, sync window, and account removal (F1.8).
///
/// The [AccountsBloc] is resolved from [context] *before* the dialog
/// route exists — dialog routes live above the shell's providers, so a
/// bloc lookup inside them would miss.
Future<void> showAccountSettingsDialog(
  BuildContext context,
  AccountSummary account,
) {
  final shellBloc = context.read<AccountsBloc>();
  return showDialog<void>(
    context: context,
    builder: (_) =>
        _AccountSettingsDialog(account: account, shellBloc: shellBloc),
  );
}

/// Sync windows the daemon's wire can express, as label → days
/// (PRD F2.2 offers 30 d / 90 d / 1 y / all; "all" needs NULL support
/// in `update_account` that the daemon does not wire yet, so the
/// dialog offers the concrete windows until that lands).
const _syncWindows = {'30 days': 30, '90 days': 90, '1 year': 365};

class _AccountSettingsDialog extends StatefulWidget {
  const _AccountSettingsDialog({
    required this.account,
    required this.shellBloc,
  });

  final AccountSummary account;

  /// Shell bloc, re-queried after every mutation.
  final AccountsBloc shellBloc;

  @override
  State<_AccountSettingsDialog> createState() => _AccountSettingsDialogState();
}

class _AccountSettingsDialogState extends State<_AccountSettingsDialog> {
  late final TextEditingController _name;
  String? _color;
  late int _syncWindowDays;
  bool _confirmingRemove = false;
  bool _working = false;
  String? _error;

  @override
  void initState() {
    super.initState();
    _name = TextEditingController(text: widget.account.displayName);
    _color = widget.account.color;
    _syncWindowDays = widget.account.syncWindowDays;
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    if (_confirmingRemove) {
      return _buildRemoveConfirmation(context, tokens);
    }
    return AlertDialog(
      key: const Key('account-settings-dialog'),
      title: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text('Account settings'),
          Text(
            widget.account.email,
            style: Theme.of(context).textTheme.bodySmall
                ?.copyWith(color: tokens.muted),
          ),
        ],
      ),
      content: SizedBox(
        width: 380,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            TextField(
              key: const Key('account-name-field'),
              controller: _name,
              enabled: !_working,
              decoration: const InputDecoration(
                labelText: 'Display name',
                isDense: true,
              ),
            ),
            const SizedBox(height: 16),
            Text(
              'Color',
              style: Theme.of(context).textTheme.labelSmall
                  ?.copyWith(color: tokens.muted),
            ),
            const SizedBox(height: 8),
            Wrap(
              spacing: 8,
              children: [
                for (final hex in accountPaletteHexes)
                  _ColorSwatch(
                    hex: hex,
                    selected: _color == hex,
                    onPressed: _working
                        ? null
                        : () => setState(() => _color = hex),
                  ),
                _ColorSwatch(
                  hex: null,
                  selected: _color == null,
                  onPressed: _working
                      ? null
                      : () => setState(() => _color = null),
                ),
              ],
            ),
            const SizedBox(height: 16),
            DropdownButtonFormField<int>(
              key: const Key('account-sync-window'),
              initialValue: _syncWindowDays,
              isExpanded: true,
              decoration: const InputDecoration(
                labelText: 'Sync window',
                isDense: true,
              ),
              items: [
                // A stored window outside the offered set still needs an
                // item to show; DropdownButtonFormField asserts when
                // its value has no matching entry.
                if (!_syncWindows.values.contains(_syncWindowDays))
                  DropdownMenuItem(
                    value: _syncWindowDays,
                    child: Text('$_syncWindowDays days'),
                  ),
                for (final entry in _syncWindows.entries)
                  DropdownMenuItem(value: entry.value, child: Text(entry.key)),
              ],
              onChanged: _working
                  ? null
                  : (days) => setState(() => _syncWindowDays = days ?? 365),
            ),
            if (_error != null) ...[
              const SizedBox(height: 12),
              Text(
                _error!,
                style: Theme.of(context).textTheme.bodySmall
                    ?.copyWith(color: tokens.error),
              ),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          key: const Key('account-remove'),
          onPressed: _working
              ? null
              : () => setState(() => _confirmingRemove = true),
          style: TextButton.styleFrom(foregroundColor: tokens.error),
          child: const Text('Remove account'),
        ),
        TextButton(
          key: const Key('account-cancel'),
          onPressed: _working ? null : () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: const Key('account-save'),
          onPressed: _working ? null : _save,
          child: _working
              ? const SizedBox(
                  width: 14,
                  height: 14,
                  child: CircularProgressIndicator(strokeWidth: 2),
                )
              : const Text('Save'),
        ),
      ],
    );
  }

  Widget _buildRemoveConfirmation(BuildContext context, AppTokens tokens) {
    return AlertDialog(
      key: const Key('account-remove-confirmation'),
      title: const Text('Remove account?'),
      content: Text(
        // F1.8: keep/delete local data is a daemon choice that is not
        // wired yet; removal always keeps synced mail on the machine.
        '${widget.account.email} stops syncing, and the mail already '
        'synced to this machine is kept.',
      ),
      actions: [
        TextButton(
          key: const Key('account-remove-cancel'),
          onPressed: () => setState(() => _confirmingRemove = false),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: const Key('account-remove-confirm'),
          onPressed: _working ? null : _remove,
          style: FilledButton.styleFrom(
            backgroundColor: tokens.error,
            foregroundColor: Colors.white,
          ),
          child: _working
              ? const SizedBox(
                  width: 14,
                  height: 14,
                  child: CircularProgressIndicator(strokeWidth: 2),
                )
              : const Text('Remove'),
        ),
      ],
    );
  }

  Future<void> _save() async {
    final trimmed = _name.text.trim();
    if (trimmed.isEmpty) {
      setState(() => _error = 'Display name cannot be empty');
      return;
    }
    setState(() {
      _working = true;
      _error = null;
    });
    final patch = AccountPatch(
      displayName: trimmed == widget.account.displayName ? null : trimmed,
      color: _color == widget.account.color ? null : _color,
      clearColor: _color == null && widget.account.color != null,
      syncWindowDays: _syncWindowDays == widget.account.syncWindowDays
          ? null
          : _syncWindowDays,
    );
    try {
      await getIt<FfiClient>().updateAccount(
        accountId: widget.account.id,
        patch: patch,
      );
      widget.shellBloc.add(const AccountsLoadRequested());
      if (mounted) Navigator.of(context).pop();
    } on AppFailure catch (failure) {
      _failed(failure.message);
    } catch (_) {
      _failed('Could not save — try again');
    }
  }

  Future<void> _remove() async {
    setState(() {
      _working = true;
      _error = null;
    });
    try {
      await getIt<FfiClient>().removeAccount(accountId: widget.account.id);
      widget.shellBloc.add(const AccountsLoadRequested());
      if (mounted) Navigator.of(context).pop();
    } on AppFailure catch (failure) {
      _failed(failure.message);
    } catch (_) {
      _failed('Could not remove — try again');
    }
  }

  void _failed(String message) {
    if (!mounted) return;
    setState(() {
      _working = false;
      _error = message;
    });
  }
}

/// One palette choice in the color row: a filled circle for a pastel,
/// a hollow one for the "no accent" option.
class _ColorSwatch extends StatelessWidget {
  const _ColorSwatch({
    required this.hex,
    required this.selected,
    this.onPressed,
  });

  /// Palette hex this swatch sets; null is the "no accent" choice.
  final String? hex;

  /// Whether this swatch matches the pending color.
  final bool selected;

  /// Selects the swatch; null while a save is in flight.
  final VoidCallback? onPressed;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    final fill = parseAccountHex(hex);
    return InkWell(
      key: Key(hex == null ? 'account-color-none' : 'account-color-$hex'),
      onTap: onPressed,
      customBorder: const CircleBorder(),
      child: Container(
        width: 24,
        height: 24,
        decoration: BoxDecoration(
          shape: BoxShape.circle,
          color: fill,
          border: Border.all(
            color: selected ? tokens.accent : tokens.hairline,
            width: selected ? 2 : 1,
          ),
        ),
        child: fill == null
            ? Icon(Symbols.block, size: 14, color: tokens.muted)
            : null,
      ),
    );
  }
}
