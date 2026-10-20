import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/features/accounts/account_gate_bloc.dart';

import 'support/fake_ffi_client.dart';

void main() {
  group('account gate', () {
    test('starts checking', () {
      final bloc = AccountGateBloc(FakeFfiClient());
      addTearDown(bloc.close);
      expect(bloc.state, const AccountGateChecking());
    });

    test('reports ready with the listed accounts', () async {
      final client = FakeFfiClient(
        accounts: const [
          AccountSummary(
            id: 1,
            displayName: 'Work',
            email: 'work@example.com',
            provider: 'imap',
            authType: 'password',
            syncWindowDays: 30,
          ),
        ],
      );
      final bloc = AccountGateBloc(client);
      addTearDown(bloc.close);

      bloc.add(const AccountGateCheckRequested());
      await bloc.stream.firstWhere((state) => state is! AccountGateChecking);

      expect(
        bloc.state,
        const AccountGateReady([
          AccountSummary(
            id: 1,
            displayName: 'Work',
            email: 'work@example.com',
            provider: 'imap',
            authType: 'password',
            syncWindowDays: 30,
          ),
        ]),
      );
    });

    test('a successful check with zero accounts wants the wizard', () async {
      final client = FakeFfiClient(accounts: const []);
      final bloc = AccountGateBloc(client);
      addTearDown(bloc.close);

      bloc.add(const AccountGateCheckRequested());
      await bloc.stream.firstWhere((state) => state is! AccountGateChecking);

      expect(bloc.state, const AccountGateNeedsAccount());
    });

    test('a failed check reports the failure message', () async {
      final client = FakeFfiClient(
        accountsFailure: const DaemonUnavailableFailure(
          message: 'cannot reach the daemon',
        ),
      );
      final bloc = AccountGateBloc(client);
      addTearDown(bloc.close);

      bloc.add(const AccountGateCheckRequested());
      await bloc.stream.firstWhere((state) => state is! AccountGateChecking);

      expect(bloc.state, const AccountGateFailure('cannot reach the daemon'));
    });

    test('a recheck after a failure lists accounts again', () async {
      final client = FakeFfiClient(
        accounts: const [
          AccountSummary(
            id: 3,
            displayName: 'Gmail',
            email: 'me@gmail.com',
            provider: 'gmail',
            authType: 'oauth2',
            syncWindowDays: 30,
          ),
        ],
      );
      final bloc = AccountGateBloc(client);
      addTearDown(bloc.close);

      client.accountsFailure = const ValidationFailure(message: 'nope');
      bloc.add(const AccountGateCheckRequested());
      await bloc.stream.firstWhere((state) => state is AccountGateFailure);

      client.accountsFailure = null;
      bloc.add(const AccountGateCheckRequested());
      await bloc.stream.firstWhere((state) => state is AccountGateReady);

      expect(client.listAccountsCalls, 2);
      expect(
        bloc.state,
        const AccountGateReady([
          AccountSummary(
            id: 3,
            displayName: 'Gmail',
            email: 'me@gmail.com',
            provider: 'gmail',
            authType: 'oauth2',
            syncWindowDays: 30,
          ),
        ]),
      );
    });

    test('states compare by value', () {
      expect(
        const AccountGateReady([]),
        isNot(
          const AccountGateReady([
            AccountSummary(
              id: 1,
              displayName: 'Work',
              email: 'work@example.com',
              provider: 'imap',
              authType: 'password',
              syncWindowDays: 30,
            ),
          ]),
        ),
      );
      expect(const AccountGateFailure('a'), const AccountGateFailure('a'));
      expect(
        const AccountGateFailure('a').hashCode,
        const AccountGateFailure('a').hashCode,
      );
    });
  });
}
