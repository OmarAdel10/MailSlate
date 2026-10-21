# accounts

Everything that gets an email account into the app and keeps it usable:
the first-launch wizard gate, the add-account wizard, the sidebar accounts
surface (list, settings, reconnect), and the account accent palette.

## Structure

- `account_gate.dart` / `account_gate_bloc.dart` — the first-launch gate:
  routes between the wizard (zero accounts) and the shell, and re-checks
  when the wizard finishes (PRD F1).
- `wizard/` — the add-account flow (PRD F2): provider presets, IMAP form
  with connection test, OAuth2 consent via the system browser. See
  `wizard/` sources for detail.
- `accounts_bloc.dart` — `AccountsBloc`: the sidebar's account list and the
  `reauth_required` set driving the reconnect cards (F1.5/F1.6). Subscribes
  to the app-wide `AppEventsBloc` state stream — never dials its own daemon
  connection (PERFORMANCE §4.5). Only one reconnect flow is pending at a
  time, so the pending account id attributes `oauth_failed` reasons to the
  card that started the flow.
- `account_settings_dialog.dart` — per-account settings (F1.5): display
  name, accent color (set or clear), sync window, and remove-with-
  confirmation (F1.8; removal keeps already-synced mail — the keep-vs-
  delete choice is not wired yet). The dialog captures the shell's
  `AccountsBloc` before opening: dialog routes sit above the shell's
  providers.
- `account_colors.dart` — the accent palette and the deterministic fallback:
  a stored `#RRGGBB` wins, else the badge pastel set indexed by account id
  (DESIGN §5.4).

## Key files

- `accounts_bloc.dart` — account list state, reconnect lifecycle.
- `account_settings_dialog.dart` — the settings/remove dialog.
- `account_colors.dart` — dot colors.
- `account_gate.dart` — wizard-vs-shell routing.

## Consumes / produces

- Consumes: `FfiClient` (`listAccounts`, `updateAccount`, `removeAccount`,
  `startReauth`, `startOauth`, `completeOauthAccount`, `testConnection`) via
  `get_it`, daemon events via `AppEventsBloc` (shell provides the stream),
  `UrlOpener` for consent pages, and design tokens via `context.tokens`.
- Produces: `AccountsBloc` (hosted by the shell), the wizard screens pushed
  as routes, and `showAccountSettingsDialog` used by the sidebar rows.

## Tests

- `test/accounts_bloc_test.dart` — list loading, event reactions
  (raise/lower/dedupe `reauth_required`, reload triggers), and the full
  reconnect lifecycle incl. every failure path.
- `test/shell_screen_test.dart` — widget-level coverage of the sidebar rows,
  settings dialog round-trips, and the reconnect cards.
- `test/account_gate_test.dart`, `test/account_gate_bloc_test.dart`,
  `test/account_wizard_*` — gate and wizard flows.
