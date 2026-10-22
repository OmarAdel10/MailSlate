import 'package:flutter/material.dart';
import 'package:flutter_bloc/flutter_bloc.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:mailslate/core/di/di.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/logging/daemon_log.dart';
import 'package:mailslate/core/logging/log_opener.dart';
import 'package:mailslate/core/system/url_opener.dart';
import 'package:mailslate/core/theme/tokens.dart';
import 'package:mailslate/features/accounts/account_colors.dart';
import 'package:mailslate/features/accounts/account_settings_dialog.dart';
import 'package:mailslate/features/accounts/accounts_bloc.dart';
import 'package:mailslate/features/accounts/wizard/account_wizard_screen.dart';

import 'app_events_bloc.dart';
import 'daemon_status_bloc.dart';

/// The app shell: window chrome around the mail views.
///
/// Sidebar 240px · conversation list 380px · reader fills (USER_FLOW §2,
/// DESIGN §5). Messages arrive with the sync milestones; M1 panes render
/// their empty states next to the live daemon connection status.
///
/// The sidebar also lists the connected accounts (F1.5) and, when the
/// daemon reports `reauth_required`, a `Reconnect <account>` card runs the
/// OAuth flow again without re-adding anything (F1.6, USER_FLOW §11).
class ShellScreen extends StatelessWidget {
  const ShellScreen({super.key, this.openThreadId});

  /// Conversation to reveal when conversation deep-linking lands (M3).
  ///
  /// Parsed from `--open-thread`; recorded so the routing seam exists before
  /// threads do.
  final String? openThreadId;

  @override
  Widget build(BuildContext context) {
    return BlocProvider(
      create: (_) =>
          DaemonStatusBloc(getIt<FfiClient>())
            ..add(const DaemonStatusRequested()),
      child: Builder(
        builder: (context) {
          return BlocProvider(
            create: (_) => AccountsBloc(
              getIt<FfiClient>(),
              // The root AppEventsBloc owns the daemon event connection;
              // this only taps its broadcast stream (PERFORMANCE §4.5).
              context.read<AppEventsBloc>().stream,
              getIt<UrlOpener>(),
            )..add(const AccountsLoadRequested()),
            child: const _ShellView(),
          );
        },
      ),
    );
  }
}

/// Top-level mail views shown in the sidebar (USER_FLOW §2).
///
/// Important joins once a connected account exposes it (Gmail, M2) — it is
/// hidden until then, never shown as an empty state.
enum MailView {
  inbox('Inbox', Symbols.inbox),
  starred('Starred', Symbols.star),
  drafts('Drafts', Symbols.drafts),
  sent('Sent', Symbols.send),
  spam('Spam', Symbols.report),
  trash('Trash', Symbols.delete);

  const MailView(this.label, this.icon);

  /// Sidebar label, and the title of the pane's empty state.
  final String label;

  /// Sidebar glyph, 16px per DESIGN §7.3.
  final IconData icon;
}

class _ShellView extends StatefulWidget {
  const _ShellView();

  @override
  State<_ShellView> createState() => _ShellViewState();
}

class _ShellViewState extends State<_ShellView> {
  MailView _selectedView = MailView.inbox;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    return Scaffold(
      backgroundColor: tokens.canvas,
      body: Row(
        children: [
          _Sidebar(
            key: const Key('shell-sidebar'),
            selectedView: _selectedView,
            onSelect: (view) => setState(() => _selectedView = view),
          ),
          VerticalDivider(width: 1, thickness: 1, color: tokens.hairline),
          Expanded(
            child: Column(
              children: [
                const _Toolbar(key: Key('shell-toolbar')),
                Divider(height: 1, thickness: 1, color: tokens.hairline),
                const _UnavailableBanner(),
                const _ReconnectBanner(),
                Expanded(
                  child: Row(
                    children: [
                      _ConversationList(
                        key: const Key('shell-list'),
                        view: _selectedView,
                      ),
                      VerticalDivider(
                        width: 1,
                        thickness: 1,
                        color: tokens.hairline,
                      ),
                      const Expanded(
                        child: _ReaderPane(key: Key('shell-reader')),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _Sidebar extends StatelessWidget {
  const _Sidebar({
    super.key,
    required this.selectedView,
    required this.onSelect,
  });

  final MailView selectedView;
  final ValueChanged<MailView> onSelect;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    return Container(
      width: 240,
      color: tokens.surfaceSoft,
      padding: const EdgeInsets.fromLTRB(12, 12, 12, 8),
      child: BlocBuilder<AccountsBloc, AccountsState>(
        builder: (context, state) {
          return ListView(
            padding: EdgeInsets.zero,
            children: [
              for (final view in MailView.values)
                _SidebarItem(
                  view: view,
                  selected: view == selectedView,
                  onTap: () => onSelect(view),
                ),
              const SizedBox(height: 8),
              if (state.accounts.isNotEmpty) ...[
                Padding(
                  padding: const EdgeInsets.fromLTRB(8, 4, 8, 6),
                  child: Text(
                    'Accounts',
                    style: Theme.of(context).textTheme.labelSmall
                        ?.copyWith(color: tokens.muted),
                  ),
                ),
                for (final account in state.accounts)
                  _AccountRow(
                    account: account,
                    needsReauth: state.reauthRequired.contains(account.id),
                  ),
                const SizedBox(height: 8),
              ],
              _AddAccountItem(
                onTap: () => Navigator.of(context)
                    .push(
                      MaterialPageRoute<void>(
                        builder: (_) => AccountWizardScreen(
                          firstAccount: false,
                          onDone: () => Navigator.of(context).pop(),
                        ),
                      ),
                    )
                    .then((_) {
                      // The wizard may have added an account (or been
                      // cancelled); refresh the sidebar either way.
                      if (context.mounted) {
                        context.read<AccountsBloc>().add(
                          const AccountsLoadRequested(),
                        );
                      }
                    }),
              ),
            ],
          );
        },
      ),
    );
  }
}

/// One selectable mail view in the sidebar (USER_FLOW §2).
class _SidebarItem extends StatelessWidget {
  const _SidebarItem({
    required this.view,
    required this.selected,
    required this.onTap,
  });

  final MailView view;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    final labelStyle = Theme.of(context).textTheme.labelMedium
        ?.copyWith(color: selected ? tokens.ink : tokens.body);
    return Padding(
      padding: const EdgeInsets.only(bottom: 2),
      child: Material(
        color: selected ? tokens.accentTint : Colors.transparent,
        borderRadius: BorderRadius.circular(8),
        child: InkWell(
          onTap: onTap,
          borderRadius: BorderRadius.circular(8),
          hoverColor: tokens.surfaceCard,
          child: SizedBox(
            height: 32,
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 8),
              child: Row(
                children: [
                  Icon(
                    view.icon,
                    size: 16,
                    color: selected ? tokens.ink : tokens.muted,
                  ),
                  const SizedBox(width: 8),
                  Text(view.label, style: labelStyle),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// One account under the sidebar's `Accounts` group (USER_FLOW §2).
///
/// M2 scope: the row and its gear open account settings (F1.5/F1.8);
/// per-account filtering of the conversation list arrives with M3. The
/// warning glyph marks accounts the daemon flagged `reauth_required`.
class _AccountRow extends StatelessWidget {
  const _AccountRow({required this.account, required this.needsReauth});

  final AccountSummary account;
  final bool needsReauth;

  void _openSettings(BuildContext context) {
    showAccountSettingsDialog(context, account);
  }

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    final labelStyle = Theme.of(context).textTheme.labelMedium
        ?.copyWith(color: needsReauth ? tokens.error : tokens.body);
    return Padding(
      padding: const EdgeInsets.only(bottom: 2),
      child: Material(
        color: Colors.transparent,
        borderRadius: BorderRadius.circular(8),
        child: InkWell(
          key: Key('account-row-${account.id}'),
          onTap: () => _openSettings(context),
          borderRadius: BorderRadius.circular(8),
          hoverColor: tokens.surfaceCard,
          child: SizedBox(
            height: 32,
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 8),
              child: Row(
                children: [
                  Container(
                    width: 10,
                    height: 10,
                    decoration: BoxDecoration(
                      color: accountDotColor(account, tokens),
                      shape: BoxShape.circle,
                    ),
                  ),
                  const SizedBox(width: 8),
                  Expanded(child: Text(account.displayName, style: labelStyle)),
                  if (needsReauth)
                    Tooltip(
                      message: 'Reconnect needed',
                      child: Icon(
                        Symbols.warning,
                        size: 16,
                        color: tokens.error,
                      ),
                    ),
                  IconButton(
                    key: Key('account-settings-${account.id}'),
                    onPressed: () => _openSettings(context),
                    icon: Icon(Symbols.settings, size: 16, color: tokens.muted),
                    tooltip: 'Account settings',
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// The `+ Add account` affordance under the mail views (USER_FLOW §12):
/// opens the wizard in second-account mode, which skips the welcome.
class _AddAccountItem extends StatelessWidget {
  const _AddAccountItem({required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    final labelStyle = Theme.of(context).textTheme.labelMedium
        ?.copyWith(color: tokens.muted);
    return Material(
      color: Colors.transparent,
      borderRadius: BorderRadius.circular(8),
      child: InkWell(
        key: const Key('sidebar-add-account'),
        onTap: onTap,
        borderRadius: BorderRadius.circular(8),
        hoverColor: tokens.surfaceCard,
        child: SizedBox(
          height: 32,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 8),
            child: Row(
              children: [
                Icon(Symbols.add, size: 16, color: tokens.muted),
                const SizedBox(width: 8),
                Text('Add account', style: labelStyle),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _Toolbar extends StatelessWidget {
  const _Toolbar({super.key});

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    return Container(
      height: 56,
      color: tokens.surfaceSoft,
      padding: const EdgeInsets.symmetric(horizontal: 16),
      child: const Row(
        children: [_DaemonStatusCluster(), Spacer(), _RefreshButton()],
      ),
    );
  }
}

class _DaemonStatusCluster extends StatelessWidget {
  const _DaemonStatusCluster();

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    final labelStyle = Theme.of(context).textTheme.labelSmall;
    return BlocBuilder<DaemonStatusBloc, DaemonStatusState>(
      builder: (context, state) {
        switch (state) {
          case DaemonConnecting():
            return Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                SizedBox(
                  width: 12,
                  height: 12,
                  child: CircularProgressIndicator(
                    strokeWidth: 2,
                    color: tokens.muted,
                  ),
                ),
                const SizedBox(width: 8),
                Text('Connecting…', style: labelStyle),
              ],
            );
          case DaemonConnected(:final daemonVersion):
            return Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                _StatusDot(color: tokens.success),
                const SizedBox(width: 8),
                Text(
                  daemonVersion == null
                      ? 'Daemon connected'
                      : 'Daemon v$daemonVersion',
                  style: labelStyle,
                ),
              ],
            );
          case DaemonUnavailable():
            return Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                _StatusDot(color: tokens.error),
                const SizedBox(width: 8),
                Text('Sync paused', style: labelStyle),
              ],
            );
        }
      },
    );
  }
}

class _RefreshButton extends StatelessWidget {
  const _RefreshButton();

  @override
  Widget build(BuildContext context) {
    return IconButton(
      onPressed: () =>
          context.read<DaemonStatusBloc>().add(const DaemonStatusRequested()),
      tooltip: 'Check daemon',
      icon: const Icon(Symbols.refresh, size: 18),
    );
  }
}

class _UnavailableBanner extends StatelessWidget {
  const _UnavailableBanner();

  @override
  Widget build(BuildContext context) {
    return BlocBuilder<DaemonStatusBloc, DaemonStatusState>(
      builder: (context, state) {
        if (state is! DaemonUnavailable) {
          return const SizedBox.shrink();
        }
        final tokens = context.tokens;
        // Fixed copy per USER_FLOW §9; the failure detail stays in logs.
        const message = 'Sync paused — daemon unavailable';
        return Padding(
          padding: const EdgeInsets.fromLTRB(16, 12, 16, 4),
          child: Container(
            padding: const EdgeInsets.fromLTRB(16, 8, 8, 8),
            decoration: BoxDecoration(
              color: tokens.surfaceCard,
              borderRadius: BorderRadius.circular(12),
              border: Border.all(color: tokens.hairline),
            ),
            child: Row(
              children: [
                Icon(Symbols.cloud_off, size: 20, color: tokens.error),
                const SizedBox(width: 12),
                Expanded(
                  child: Text(
                    message,
                    style: Theme.of(context).textTheme.bodySmall,
                  ),
                ),
                const SizedBox(width: 12),
                OutlinedButton(
                  onPressed: () => context.read<DaemonStatusBloc>().add(
                    const DaemonStatusRequested(),
                  ),
                  child: const Text('Retry'),
                ),
                const SizedBox(width: 8),
                const _ViewLogButton(),
              ],
            ),
          ),
        );
      },
    );
  }
}

/// Inline `Reconnect <account>` cards for accounts the daemon flagged
/// `reauth_required` (USER_FLOW §11, PRD F9.4). Reader and list keep
/// working; the card re-runs the OAuth flow in the background (F1.6).
class _ReconnectBanner extends StatelessWidget {
  const _ReconnectBanner();

  @override
  Widget build(BuildContext context) {
    return BlocBuilder<AccountsBloc, AccountsState>(
      builder: (context, state) {
        if (state.reauthRequired.isEmpty) {
          return const SizedBox.shrink();
        }
        return Padding(
          padding: const EdgeInsets.fromLTRB(16, 12, 16, 4),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (final id in state.reauthRequired)
                Padding(
                  padding: const EdgeInsets.only(bottom: 4),
                  child: _ReconnectCard(
                    accountId: id,
                    displayName: _displayName(state, id),
                    pending: state.reconnectPending,
                    failure: state.reconnectFailure?.account == id
                        ? state.reconnectFailure?.reason
                        : null,
                  ),
                ),
            ],
          ),
        );
      },
    );
  }

  /// Account name for the card; `an account` covers an event that raced
  /// ahead of the account list (e.g. account added elsewhere).
  static String _displayName(AccountsState state, int accountId) {
    for (final account in state.accounts) {
      if (account.id == accountId) {
        return account.displayName;
      }
    }
    return 'an account';
  }
}

class _ReconnectCard extends StatelessWidget {
  const _ReconnectCard({
    required this.accountId,
    required this.displayName,
    required this.pending,
    required this.failure,
  });

  final int accountId;
  final String displayName;
  final int? pending;
  final String? failure;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    final bodySmall = Theme.of(context).textTheme.bodySmall;
    final waiting = pending == accountId;
    return Container(
      padding: const EdgeInsets.fromLTRB(16, 8, 8, 8),
      decoration: BoxDecoration(
        color: tokens.surfaceCard,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: tokens.hairline),
      ),
      child: Row(
        children: [
          Icon(Symbols.warning, size: 20, color: tokens.error),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text('Reconnect $displayName', style: bodySmall),
                if (failure != null)
                  Text(
                    failure!,
                    style: bodySmall?.copyWith(color: tokens.error),
                  ),
                if (waiting)
                  Text(
                    'Waiting for the browser…',
                    style: bodySmall?.copyWith(color: tokens.muted),
                  ),
              ],
            ),
          ),
          const SizedBox(width: 12),
          FilledButton(
            key: Key('account-reconnect-$accountId'),
            // The daemon rejects a second concurrent OAuth flow, so every
            // card disables while one reconnect is in flight.
            onPressed: pending == null
                ? () => context.read<AccountsBloc>().add(
                    AccountReconnectRequested(accountId),
                  )
                : null,
            child: waiting
                ? const SizedBox(
                    width: 12,
                    height: 12,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  )
                : const Text('Reconnect'),
          ),
        ],
      ),
    );
  }
}

class _ViewLogButton extends StatefulWidget {
  const _ViewLogButton();

  @override
  State<_ViewLogButton> createState() => _ViewLogButtonState();
}

/// Paused-card action opening the newest daemon log (USER_FLOW §9).
///
/// Disabled until a log file exists: the daemon has not run yet, or the
/// log directory has no rotation file.
class _ViewLogButtonState extends State<_ViewLogButton> {
  String? _newestLog;

  @override
  void initState() {
    super.initState();
    // Resolve after the first frame so the banner paints immediately.
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      String? newest;
      try {
        final dir = await getIt<FfiClient>().daemonLogDir();
        newest = dir.isEmpty ? null : newestDaemonLog(dir);
      } catch (_) {
        // The bridge is unavailable (never loaded, daemon never ran):
        // stay disabled instead of crashing the paused banner.
        newest = null;
      }
      if (!mounted) {
        return;
      }
      setState(() {
        _newestLog = newest;
      });
    });
  }

  @override
  Widget build(BuildContext context) {
    final log = _newestLog;
    return Tooltip(
      message: log == null ? 'No log file yet' : 'Open the newest daemon log',
      child: TextButton(
        key: const Key('view-log-button'),
        onPressed: log == null ? null : () => getIt<LogOpener>().open(log),
        child: const Text('View log'),
      ),
    );
  }
}

class _StatusDot extends StatelessWidget {
  const _StatusDot({required this.color});

  final Color color;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: 8,
      height: 8,
      decoration: BoxDecoration(color: color, shape: BoxShape.circle),
    );
  }
}

class _ConversationList extends StatelessWidget {
  const _ConversationList({super.key, required this.view});

  final MailView view;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: 380,
      child: _EmptyPane(title: view.label, hint: '${view.label} is empty'),
    );
  }
}

class _EmptyPane extends StatelessWidget {
  const _EmptyPane({required this.title, required this.hint});

  final String title;
  final String hint;

  @override
  Widget build(BuildContext context) {
    final textTheme = Theme.of(context).textTheme;
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(
            title,
            style: textTheme.displaySmall,
            textAlign: TextAlign.center,
          ),
          const SizedBox(height: 8),
          Text(hint, style: textTheme.bodySmall, textAlign: TextAlign.center),
        ],
      ),
    );
  }
}

class _ReaderPane extends StatelessWidget {
  const _ReaderPane({super.key});

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    return Center(
      child: Text(
        'Select a conversation',
        style: Theme.of(context).textTheme.bodyLarge
            ?.copyWith(color: tokens.muted),
      ),
    );
  }
}
