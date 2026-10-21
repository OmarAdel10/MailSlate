# shell

The app's window chrome: the 3-pane layout and the daemon-status surface that
frames every mail view (USER_FLOW §2, DESIGN §5).

## Structure

- `shell_screen.dart` — `ShellScreen`: sidebar 240px / conversation list 380px
  / reader fills, with a 56px toolbar. `MailView` enumerates the unified
  sidebar views; panes render their spec empty states until mail data lands.
- `daemon_status_bloc.dart` — `DaemonStatusBloc`: checks the daemon, spawns it
  when it is down (USER_FLOW §9), and drives the toolbar status cluster and
  the `Sync paused — daemon unavailable` banner with its Retry action.
- `app_events_bloc.dart` — `AppEventsBloc`: the app-wide daemon event stream,
  provided at the app root; the accounts surface (below) taps its state
  stream instead of dialing its own daemon connection.
- Accounts surface (from `features/accounts`), hosted by the shell:
  - the sidebar's `Accounts` group lists every connected account (F1.5) and
    opens account settings per row;
  - `_ReconnectBanner` raises an inline `Reconnect <account>` card per
    account the daemon flags `reauth_required` (F1.6, USER_FLOW §11) and
    runs the reauth flow via `AccountsBloc`.

## Key files

- `shell_screen.dart` — layout, sidebar selection + account rows, toolbar,
  paused banner, reconnect cards.
- `daemon_status_bloc.dart` — connecting / connected / unavailable states.
- `app_events_bloc.dart` — single daemon event connection for the whole app.

## Consumes / produces

- Consumes: `FfiClient` via `get_it` (`lib/core/di/di.dart`), the design
  tokens through `context.tokens` (`lib/core/theme/`), `AppEventsBloc`
  (from the app root), and the accounts feature (`features/accounts`).
- Produces: nothing outside the shell; `ShellScreen.openThreadId` records the
  `--open-thread` launch argument until conversation deep-linking lands (M3).

## Tests

- `test/shell_screen_test.dart` — pane sizes, sidebar navigation, the paused
  banner with Retry recovery, the sidebar account list, the settings dialog
  round-trips, and the reconnect card lifecycle.
- `test/daemon_status_bloc_test.dart` — connect, spawn, and failure paths,
  driven by `test/support/fake_ffi_client.dart`.
- `test/accounts_bloc_test.dart` — the accounts surface bloc in isolation.
