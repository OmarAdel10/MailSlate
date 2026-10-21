import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/features/accounts/accounts_bloc.dart';
import 'package:mailslate/features/shell/app_events_bloc.dart';

import 'support/fake_ffi_client.dart';
import 'support/fake_url_opener.dart';

/// Wraps raw events the way the shell does: through the events bloc.
Stream<AppEventsState> _live(Stream<AppEvent> events) =>
    events.map(AppEventsLive.new);

AccountSummary _account(int id, String name) => AccountSummary(
  id: id,
  displayName: name,
  email: 'user@example.com',
  provider: 'gmail',
  authType: 'oauth2',
  color: null,
  syncWindowDays: 365,
);

void main() {
  group('AccountsBloc loads', () {
    test('stores the account list', () async {
      final client = FakeFfiClient(
        accounts: [_account(1, 'Personal'), _account(2, 'Work')],
      );
      final bloc = AccountsBloc(
        client,
        const Stream<AppEventsState>.empty(),
        FakeUrlOpener(),
      );
      addTearDown(bloc.close);

      bloc.add(const AccountsLoadRequested());
      await pumpEventQueue();

      expect(bloc.state.accounts, hasLength(2));
      expect(bloc.state.loading, isFalse);
      expect(bloc.state.loadFailure, isNull);
      expect(client.listAccountsCalls, 1);
    });

    test('records an AppFailure message', () async {
      final client = FakeFfiClient(accountsFailure: const NetworkFailure());
      final bloc = AccountsBloc(
        client,
        const Stream<AppEventsState>.empty(),
        FakeUrlOpener(),
      );
      addTearDown(bloc.close);

      bloc.add(const AccountsLoadRequested());
      await pumpEventQueue();

      expect(bloc.state.accounts, isEmpty);
      expect(bloc.state.loadFailure, 'Connection problem — check your network');
      expect(bloc.state.loading, isFalse);
    });

    test('records a generic message for unexpected errors', () async {
      final client = FakeFfiClient(accounts: const [])
        ..listAccountsError = StateError('bridge died');
      final bloc = AccountsBloc(
        client,
        const Stream<AppEventsState>.empty(),
        FakeUrlOpener(),
      );
      addTearDown(bloc.close);

      bloc.add(const AccountsLoadRequested());
      await pumpEventQueue();

      expect(bloc.state.loadFailure, 'Could not load accounts');
    });
  });

  group('AccountsBloc reacts to daemon events', () {
    late StreamController<AppEvent> events;
    late FakeFfiClient client;
    late AccountsBloc bloc;

    setUp(() {
      events = StreamController<AppEvent>();
      client = FakeFfiClient(accounts: [_account(1, 'Personal')]);
      bloc = AccountsBloc(client, _live(events.stream), FakeUrlOpener());
      addTearDown(() async {
        await bloc.close();
        await events.close();
      });
    });

    test('account_status raises and lowers the reconnect set', () async {
      events.add(
        const AppEvent.accountStatus(account: 1, state: 'reauth_required'),
      );
      await pumpEventQueue();

      expect(bloc.state.reauthRequired, {1});

      events.add(const AppEvent.accountStatus(account: 1, state: 'syncing'));
      await pumpEventQueue();

      expect(bloc.state.reauthRequired, isEmpty);
    });

    test('duplicate reauth_required events emit nothing new', () async {
      var emissions = 0;
      final subscription = bloc.stream.listen((_) => emissions += 1);
      addTearDown(subscription.cancel);

      events.add(
        const AppEvent.accountStatus(account: 1, state: 'reauth_required'),
      );
      await pumpEventQueue();
      expect(emissions, 1);

      events.add(
        const AppEvent.accountStatus(account: 1, state: 'reauth_required'),
      );
      await pumpEventQueue();
      expect(emissions, 1);
      expect(bloc.state.reauthRequired, {1});
    });

    test('oauth_completed reloads the account list', () async {
      client.listAccountsCalls = 0;
      events.add(const AppEvent.oauthCompleted(email: 'a@b.c'));
      await pumpEventQueue();

      expect(client.listAccountsCalls, 1);
    });

    test('unrelated events change nothing', () async {
      events.add(
        const AppEvent.syncProgress(
          account: 1,
          mailbox: 'INBOX',
          phase: 'initial',
          done: 1,
          total: 2,
        ),
      );
      events.add(const AppEvent.oauthUrlReady(url: 'https://x.test'));
      events.add(const AppEvent.other(name: 'something_new'));
      await pumpEventQueue();

      expect(bloc.state.reauthRequired, isEmpty);
      expect(client.listAccountsCalls, 0);
    });
  });

  group('AccountReconnectRequested', () {
    late StreamController<AppEvent> events;
    late FakeUrlOpener opener;
    late FakeFfiClient client;
    late AccountsBloc bloc;

    setUp(() {
      events = StreamController<AppEvent>();
      opener = FakeUrlOpener();
      client = FakeFfiClient(accounts: [_account(7, 'Gmail')]);
      bloc = AccountsBloc(client, _live(events.stream), opener);
      addTearDown(() async {
        await bloc.close();
        await events.close();
      });
    });

    test('starts the flow and stays pending until oauth_reconnected', () async {
      events.add(
        const AppEvent.accountStatus(account: 7, state: 'reauth_required'),
      );
      await pumpEventQueue();

      bloc.add(const AccountReconnectRequested(7));
      await pumpEventQueue();

      expect(client.reauthCalls, 1);
      expect(client.lastReauthId, 7);
      expect(opener.openedUrls, [client.reauthUrl]);
      // Pending: the flow finishes in the daemon, not here.
      expect(bloc.state.reconnectPending, 7);

      events.add(
        const AppEvent.oauthReconnected(account: 7, email: 'user@example.com'),
      );
      await pumpEventQueue();

      expect(bloc.state.reconnectPending, isNull);
      expect(bloc.state.reconnectFailure, isNull);
      expect(bloc.state.reauthRequired, isEmpty);
      // The reconnect reloads the account list.
      expect(client.listAccountsCalls, greaterThan(0));
    });

    test('ignores a second request while one flow is pending', () async {
      events.add(
        const AppEvent.accountStatus(account: 7, state: 'reauth_required'),
      );
      await pumpEventQueue();

      bloc.add(const AccountReconnectRequested(7));
      await pumpEventQueue();
      bloc.add(const AccountReconnectRequested(7));
      await pumpEventQueue();

      expect(client.reauthCalls, 1);
      expect(bloc.state.reconnectPending, 7);
    });

    test('records an AppFailure message on the failing account', () async {
      client.reauthFailure = const AuthFailedFailure();

      bloc.add(const AccountReconnectRequested(7));
      await pumpEventQueue();

      expect(bloc.state.reconnectPending, isNull);
      expect(bloc.state.reconnectFailure, (
        account: 7,
        reason: 'Sign-in failed — check your credentials',
      ));
    });

    test('records a hint when the browser cannot open', () async {
      opener.openResult = false;

      bloc.add(const AccountReconnectRequested(7));
      await pumpEventQueue();

      expect(bloc.state.reconnectPending, isNull);
      expect(bloc.state.reconnectFailure, (
        account: 7,
        reason: 'Could not open the browser',
      ));
    });

    test('records a generic reason on unexpected errors', () async {
      client.reauthThrow = StateError('bridge died');

      bloc.add(const AccountReconnectRequested(7));
      await pumpEventQueue();

      expect(bloc.state.reconnectFailure, (
        account: 7,
        reason: 'Could not start reconnect — try again',
      ));
    });

    test(
      'oauth_failed while pending clears pending and keeps the card',
      () async {
        events.add(
          const AppEvent.accountStatus(account: 7, state: 'reauth_required'),
        );
        await pumpEventQueue();

        bloc.add(const AccountReconnectRequested(7));
        await pumpEventQueue();

        events.add(
          const AppEvent.oauthFailed(reason: 'token exchange rejected'),
        );
        await pumpEventQueue();

        expect(bloc.state.reconnectPending, isNull);
        expect(bloc.state.reconnectFailure, (
          account: 7,
          reason: 'token exchange rejected',
        ));
        // The grant is still dead: the card stays up for another try.
        expect(bloc.state.reauthRequired, {7});
      },
    );

    test('oauth_failed without a pending reconnect is ignored', () async {
      events.add(const AppEvent.oauthFailed(reason: 'wizard failure'));
      await pumpEventQueue();

      expect(bloc.state.reconnectFailure, isNull);
      expect(bloc.state.reconnectPending, isNull);
    });
  });
}
