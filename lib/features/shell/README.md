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

## Key files

- `shell_screen.dart` — layout, sidebar selection, toolbar, paused banner.
- `daemon_status_bloc.dart` — connecting / connected / unavailable states.

## Consumes / produces

- Consumes: `FfiClient` via `get_it` (`lib/core/di/di.dart`) and the design
  tokens through `context.tokens` (`lib/core/theme/`).
- Produces: nothing outside the shell; `ShellScreen.openThreadId` records the
  `--open-thread` launch argument until conversation deep-linking lands (M3).

## Tests

- `test/shell_screen_test.dart` — pane sizes, sidebar navigation, and the
  paused banner with Retry recovery.
- `test/daemon_status_bloc_test.dart` — connect, spawn, and failure paths,
  driven by `test/support/fake_ffi_client.dart`.
