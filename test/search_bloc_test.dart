import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/features/search/search_bloc.dart';
import 'package:mailslate/src/rust/api/search.dart' as rust_search;

import 'support/fake_ffi_client.dart';

/// The daemon reply the fake default: one hit, total one.
rust_search.SearchResult _result({
  List<rust_search.SearchHit>? messages,
  int total = 1,
}) => rust_search.SearchResult(
  messages:
      messages ??
      const [
        rust_search.SearchHit(
          messageId: 1,
          accountId: 42,
          subject: 'Work update',
          snippet: 'body text',
        ),
      ],
  total: total,
);

/// Drains the bloc queues until `done` holds, so the 150ms debounce
/// timer and any queued interim events all finish processing.
Future<void> _settle(SearchBloc bloc, bool Function() done) async {
  for (var round = 0; round < 400; round++) {
    await pumpEventQueue();
    await Future<void>.delayed(const Duration(milliseconds: 50));
    await pumpEventQueue();
    if (done()) return;
  }
  fail('search bloc never reached the expected state');
}

void main() {
  group('SearchBloc idles', () {
    test('before any query', () {
      final bloc = SearchBloc(FakeFfiClient());
      addTearDown(bloc.close);
      expect(bloc.state, isA<SearchIdle>());
    });
  });

  group('SearchBloc runs a query', () {
    test('after the debounce, with results', () async {
      final client = FakeFfiClient(searchResult: _result());
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(
        bloc,
        () => bloc.state is SearchResults && client.searchCalls == 1,
      );

      final state = bloc.state as SearchResults;
      expect(state.query, 'budgets');
      expect(state.total, 1);
      expect(state.hits, hasLength(1));
      expect(state.hits.single.messageId, 1);
      expect(state.hits.single.accountId, 42);
      expect(state.hits.single.subject, 'Work update');
      expect(state.hits.single.snippet, 'body text');
      expect(client.searchCalls, 1);
      expect(client.lastSearchQuery, 'budgets');
    });

    test('searches across every account', () async {
      final client = FakeFfiClient();
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('hi'));
      await _settle(bloc, () => client.searchCalls == 1);

      expect(client.searchCalls, 1);
      expect(client.lastSearchAccountId, -1);
    });

    test('drops stale keystrokes and searches the latest once', () async {
      final client = FakeFfiClient();
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('a'));
      bloc.add(const SearchQueryChanged('ab'));
      bloc.add(const SearchQueryChanged('abc'));
      await _settle(
        bloc,
        () =>
            bloc.state is SearchResults &&
            client.searchCalls == 1 &&
            (bloc.state as SearchResults).query == 'abc',
      );

      final state = bloc.state as SearchResults;
      expect(state.query, 'abc');
      expect(client.searchCalls, 1);
      expect(client.lastSearchQuery, 'abc');
    });

    test('maps a daemon failure into the error state', () async {
      final client = FakeFfiClient(
        searchFailure: const NetworkFailure(message: 'daemon gone'),
      );
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => bloc.state is SearchErrorState);

      expect(bloc.state, isA<SearchErrorState>());
      expect((bloc.state as SearchErrorState).message, 'daemon gone');
    });

    test('maps an unexpected throw into the fallback message', () async {
      final client = FakeFfiClient(searchThrow: StateError('boom'));
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => bloc.state is SearchErrorState);

      final state = bloc.state as SearchErrorState;
      expect(state.message, 'Search failed — try again.');
    });
  });

  group('SearchBloc toggles filters', () {
    const tag = SearchFilter(key: 'tag', value: 'invoices');

    test('with no query stays idle and never searches', () async {
      final client = FakeFfiClient();
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchFilterToggled(tag));
      await _settle(bloc, () => bloc.state is SearchIdle);

      expect(bloc.state, isA<SearchIdle>());
      expect(client.searchCalls, 0);
    });

    test('re-runs the search with the filter clause', () async {
      final client = FakeFfiClient();
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => client.searchCalls == 1);
      expect(client.searchCalls, 1);

      bloc.add(const SearchFilterToggled(tag));
      await _settle(
        bloc,
        () => client.searchCalls == 2 && bloc.state is SearchResults,
      );

      expect(client.searchCalls, 2);
      expect(client.lastSearchQuery, 'budgets tag:invoices');
      expect(bloc.state, isA<SearchResults>());
    });

    test('same key toggled twice removes the filter', () async {
      final client = FakeFfiClient();
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => client.searchCalls == 1);
      bloc.add(const SearchFilterToggled(tag));
      await _settle(bloc, () => client.searchCalls == 2);
      bloc.add(const SearchFilterToggled(tag));
      await _settle(bloc, () => client.searchCalls == 3);

      expect(client.searchCalls, 3);
      expect(client.lastSearchQuery, 'budgets');
      expect(bloc.state, isA<SearchResults>());
    });

    test('maps a daemon failure during a filter re-run', () async {
      final client = FakeFfiClient();
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => client.searchCalls == 1);
      client.searchFailure = const NetworkFailure(message: 'daemon gone');

      bloc.add(const SearchFilterToggled(tag));
      await _settle(bloc, () => bloc.state is SearchErrorState);

      expect((bloc.state as SearchErrorState).message, 'daemon gone');
      expect(client.searchCalls, 2);
    });

    test('maps an unexpected throw during a filter re-run', () async {
      final client = FakeFfiClient();
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => client.searchCalls == 1);
      client.searchThrow = StateError('boom');

      bloc.add(const SearchFilterToggled(tag));
      await _settle(bloc, () => bloc.state is SearchErrorState);

      expect(
        (bloc.state as SearchErrorState).message,
        'Search failed — try again.',
      );
      expect(client.searchCalls, 2);
    });
  });

  group('SearchBloc clears', () {
    test('clearing back to empty goes idle without searching', () async {
      final client = FakeFfiClient();
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => bloc.state is SearchResults);

      bloc.add(const SearchQueryChanged(''));
      await _settle(bloc, () => bloc.state is SearchIdle);

      // Only the first, non-empty query ran.
      expect(client.searchCalls, 1);
    });

    test('returns to idle and drops filters', () async {
      final client = FakeFfiClient();
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => bloc.state is SearchResults);

      bloc.add(const SearchCleared());
      await _settle(bloc, () => bloc.state is SearchIdle);
      expect(bloc.state, isA<SearchIdle>());

      // Filters were cleared with the clear event: a retro filter toggle
      // with no query cannot resurrect a search.
      bloc.add(const SearchFilterToggled(SearchFilter(key: 'tag', value: 'x')));
      await _settle(bloc, () => bloc.state is SearchIdle);
      expect(bloc.state, isA<SearchIdle>());
      expect(client.searchCalls, 1);
    });

    test('an empty query inside the debounce window never searches', () async {
      final client = FakeFfiClient();
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('x'));
      bloc.add(const SearchQueryChanged(''));
      await _settle(bloc, () => bloc.state is SearchIdle);

      expect(bloc.state, isA<SearchIdle>());
      expect(client.searchCalls, 0);
    });

    test(
      'a mid-flight clear keeps the bloc idle once the gate opens',
      () async {
        final gate = Completer<void>();
        final client = FakeFfiClient(searchGate: gate);
        final bloc = SearchBloc(client);
        addTearDown(bloc.close);

        bloc.add(const SearchQueryChanged('budgets'));
        await _settle(bloc, () => client.searchCalls == 1);

        bloc.add(const SearchQueryChanged(''));
        await _settle(bloc, () => bloc.state is SearchIdle);

        gate.complete();
        await pumpEventQueue();
        await Future<void>.delayed(const Duration(milliseconds: 50));
        await pumpEventQueue();

        // The stale in-flight handler must not overwrite the idle state.
        expect(bloc.state, isA<SearchIdle>());
        expect(client.searchCalls, 1);
      },
    );

    test(
      'a mid-flight SearchCleared keeps the bloc idle once the gate opens',
      () async {
        final gate = Completer<void>();
        final client = FakeFfiClient(searchGate: gate);
        final bloc = SearchBloc(client);
        addTearDown(bloc.close);

        bloc.add(const SearchQueryChanged('budgets'));
        await _settle(bloc, () => client.searchCalls == 1);

        bloc.add(const SearchCleared());
        await _settle(bloc, () => bloc.state is SearchIdle);

        gate.complete();
        await pumpEventQueue();
        await Future<void>.delayed(const Duration(milliseconds: 50));
        await pumpEventQueue();

        expect(bloc.state, isA<SearchIdle>());
        expect(client.searchCalls, 1);
      },
    );
  });

  group('SearchBloc submit', () {
    test('is a no-op while idle', () async {
      final client = FakeFfiClient();
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchSubmitted());
      await pumpEventQueue();

      expect(bloc.state, isA<SearchIdle>());
      expect(client.searchCalls, 0);
    });

    test('does not double-fire a debounced query', () async {
      final client = FakeFfiClient();
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      bloc.add(const SearchSubmitted());
      await _settle(bloc, () => bloc.state is SearchResults);

      expect(client.searchCalls, 1);
      expect(bloc.state, isA<SearchResults>());
    });
  });
}
