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

/// Opens the gate and drains the queues so parked handlers resume.
Future<void> _openGate(Completer<void> gate) async {
  gate.complete();
  await pumpEventQueue();
  await Future<void>.delayed(const Duration(milliseconds: 50));
  await pumpEventQueue();
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

    test(
      'a superseded in-flight query never emits once the gate opens',
      () async {
        final gate = Completer<void>();
        final client = FakeFfiClient(searchGate: gate);
        final bloc = SearchBloc(client);
        addTearDown(bloc.close);

        bloc.add(const SearchQueryChanged('a'));
        await _settle(bloc, () => client.searchCalls == 1);
        bloc.add(const SearchQueryChanged('ab'));
        await _settle(bloc, () => client.searchCalls == 2);

        await _openGate(gate);

        // The newer query wins; the parked run for `a` is stale.
        final state = bloc.state as SearchResults;
        expect(state.query, 'ab');
        expect(client.searchCalls, 2);
        expect(client.lastSearchQuery, 'ab');
      },
    );

    test(
      'a superseded query failure surfaces only for the newer one',
      () async {
        final gate = Completer<void>();
        final client = FakeFfiClient(searchGate: gate);
        final bloc = SearchBloc(client);
        addTearDown(bloc.close);

        bloc.add(const SearchQueryChanged('a'));
        await _settle(bloc, () => client.searchCalls == 1);
        client.searchFailureAfterGate = const NetworkFailure(
          message: 'daemon gone',
        );
        bloc.add(const SearchQueryChanged('ab'));
        await _settle(bloc, () => client.searchCalls == 2);

        await _openGate(gate);

        expect(bloc.state, isA<SearchErrorState>());
        expect((bloc.state as SearchErrorState).message, 'daemon gone');
        expect(client.lastSearchQuery, 'ab');
      },
    );

    test('a superseded query throw surfaces only for the newer one', () async {
      final gate = Completer<void>();
      final client = FakeFfiClient(searchGate: gate);
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('a'));
      await _settle(bloc, () => client.searchCalls == 1);
      client.searchThrowAfterGate = StateError('boom');
      bloc.add(const SearchQueryChanged('ab'));
      await _settle(bloc, () => client.searchCalls == 2);

      await _openGate(gate);

      expect(bloc.state, isA<SearchErrorState>());
      expect(
        (bloc.state as SearchErrorState).message,
        'Search failed — try again.',
      );
      expect(client.lastSearchQuery, 'ab');
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

    test(
      'a mid-flight clear keeps the filter re-run idle once the gate opens',
      () async {
        final gate = Completer<void>();
        final client = FakeFfiClient(searchGate: gate);
        final bloc = SearchBloc(client);
        addTearDown(bloc.close);

        bloc.add(const SearchQueryChanged('budgets'));
        await _settle(bloc, () => client.searchCalls == 1);
        bloc.add(const SearchFilterToggled(tag));
        await _settle(bloc, () => client.searchCalls == 2);

        bloc.add(const SearchCleared());
        await _settle(bloc, () => bloc.state is SearchIdle);

        await _openGate(gate);

        // Neither the stale query run nor the stale filter re-run may
        // overwrite the idle state once the gate opens.
        expect(bloc.state, isA<SearchIdle>());
        expect(client.searchCalls, 2);
      },
    );

    test('a mid-flight clear drops the stale filter failure', () async {
      final gate = Completer<void>();
      final client = FakeFfiClient(searchGate: gate);
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => client.searchCalls == 1);
      client.searchFailureAfterGate = const NetworkFailure(
        message: 'daemon gone',
      );
      bloc.add(const SearchFilterToggled(tag));
      await _settle(bloc, () => client.searchCalls == 2);

      bloc.add(const SearchCleared());
      await _settle(bloc, () => bloc.state is SearchIdle);

      await _openGate(gate);

      // The stale filter re-run's failure must not surface.
      expect(bloc.state, isA<SearchIdle>());
      expect(client.searchCalls, 2);
    });

    test('a mid-flight clear drops the stale filter throw', () async {
      final gate = Completer<void>();
      final client = FakeFfiClient(searchGate: gate);
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => client.searchCalls == 1);
      client.searchThrowAfterGate = StateError('boom');
      bloc.add(const SearchFilterToggled(tag));
      await _settle(bloc, () => client.searchCalls == 2);

      bloc.add(const SearchCleared());
      await _settle(bloc, () => bloc.state is SearchIdle);

      await _openGate(gate);

      expect(bloc.state, isA<SearchIdle>());
      expect(client.searchCalls, 2);
    });

    test(
      'a re-toggle drops the stale re-run but keeps the fresh one',
      () async {
        final gate = Completer<void>();
        final client = FakeFfiClient(searchGate: gate);
        final bloc = SearchBloc(client);
        addTearDown(bloc.close);

        bloc.add(const SearchQueryChanged('budgets'));
        await _settle(bloc, () => client.searchCalls == 1);
        bloc.add(const SearchFilterToggled(tag));
        await _settle(bloc, () => client.searchCalls == 2);
        bloc.add(const SearchFilterToggled(tag));
        await _settle(bloc, () => client.searchCalls == 3);

        await _openGate(gate);

        // The run that added the filter is stale (the second toggle
        // removed it), so the latest filterless run is the final state.
        final state = bloc.state as SearchResults;
        expect(state.query, 'budgets');
        expect(client.searchCalls, 3);
        expect(client.lastSearchQuery, 'budgets');
      },
    );

    test('the filter re-run reports the composed query', () async {
      final client = FakeFfiClient();
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => client.searchCalls == 1);
      bloc.add(const SearchFilterToggled(tag));
      await _settle(
        bloc,
        () => client.searchCalls == 2 && bloc.state is SearchResults,
      );

      // The filter path reports the composed query, not the raw text.
      expect((bloc.state as SearchResults).query, 'budgets tag:invoices');
    });

    test('an empty-text reset keeps the filters for the next query', () async {
      final client = FakeFfiClient();
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => client.searchCalls == 1);
      bloc.add(const SearchFilterToggled(tag));
      await _settle(bloc, () => client.searchCalls == 2);

      bloc.add(const SearchQueryChanged(''));
      await _settle(bloc, () => bloc.state is SearchIdle);

      bloc.add(const SearchQueryChanged('report'));
      await _settle(bloc, () => bloc.state is SearchResults);

      expect(client.lastSearchQuery, 'report tag:invoices');
    });

    test('an empty-value filter adds no clause to the query', () async {
      final client = FakeFfiClient();
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => client.searchCalls == 1);
      bloc.add(const SearchFilterToggled(SearchFilter(key: 'tag', value: '')));
      await _settle(bloc, () => client.searchCalls == 2);

      expect(client.lastSearchQuery, 'budgets');
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

        await _openGate(gate);

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

        await _openGate(gate);

        expect(bloc.state, isA<SearchIdle>());
        expect(client.searchCalls, 1);
      },
    );

    test('a mid-flight clear drops the stale daemon failure', () async {
      final gate = Completer<void>();
      final client = FakeFfiClient(searchGate: gate);
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => client.searchCalls == 1);
      client.searchFailureAfterGate = const NetworkFailure(
        message: 'daemon gone',
      );

      bloc.add(const SearchQueryChanged(''));
      await _settle(bloc, () => bloc.state is SearchIdle);

      await _openGate(gate);

      // The stale failure must not surface after the clear.
      expect(bloc.state, isA<SearchIdle>());
      expect(client.searchCalls, 1);
    });

    test('a mid-flight clear drops the stale unexpected throw', () async {
      final gate = Completer<void>();
      final client = FakeFfiClient(searchGate: gate);
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => client.searchCalls == 1);
      client.searchThrowAfterGate = StateError('boom');

      bloc.add(const SearchQueryChanged(''));
      await _settle(bloc, () => bloc.state is SearchIdle);

      await _openGate(gate);

      expect(bloc.state, isA<SearchIdle>());
      expect(client.searchCalls, 1);
    });
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

  group('SearchBloc lifecycle', () {
    test('closing mid-debounce does not search and does not throw', () async {
      final client = FakeFfiClient();
      final bloc = SearchBloc(client);

      bloc.add(const SearchQueryChanged('budgets'));
      await bloc.close();
      await pumpEventQueue();
      await Future<void>.delayed(const Duration(milliseconds: 200));
      await pumpEventQueue();

      // The debounce delay resolves before the isDone check, so a close
      // inside the window never reaches the client.
      expect(bloc.isClosed, isTrue);
      expect(client.searchCalls, 0);
    });
  });

  group('SearchBloc cross-path races', () {
    test('an inverted filter toggle wins over the query run', () async {
      final gateA = Completer<void>();
      final gateB = Completer<void>();
      final client = FakeFfiClient();
      client.searchGates
        ..add(gateA)
        ..add(gateB);
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);
      final states = <SearchState>[];
      final subscription = bloc.stream.listen(states.add);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => client.searchCalls == 1);
      bloc.add(
        const SearchFilterToggled(SearchFilter(key: 'tag', value: 'invoices')),
      );
      await _settle(bloc, () => client.searchCalls == 2);

      // Inverted completion order: the toggle resolves first, the
      // older query run last.
      await _openGate(gateB);
      await _openGate(gateA);
      await subscription.cancel();

      expect(bloc.state, isA<SearchResults>());
      expect((bloc.state as SearchResults).query, 'budgets tag:invoices');
      expect(client.searchCalls, 2);
      // The superseded query run must never surface its unfiltered
      // hits.
      expect(
        states.whereType<SearchResults>().map((s) => s.query),
        everyElement('budgets tag:invoices'),
      );
    });

    test('an inverted other-key toggle wins over the first re-run', () async {
      final client = FakeFfiClient();
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => bloc.state is SearchResults);

      final gateA = Completer<void>();
      final gateB = Completer<void>();
      client.searchGates
        ..add(gateA)
        ..add(gateB);

      bloc.add(
        const SearchFilterToggled(SearchFilter(key: 'tag', value: 'invoices')),
      );
      await _settle(bloc, () => client.searchCalls == 2);
      bloc.add(
        const SearchFilterToggled(SearchFilter(key: 'in', value: 'inbox')),
      );
      await _settle(bloc, () => client.searchCalls == 3);

      // Inverted completion order: the two-clause re-run resolves
      // first and emits the composed results.
      await _openGate(gateB);
      expect(bloc.state, isA<SearchResults>());
      expect(
        (bloc.state as SearchResults).query,
        'budgets tag:invoices in:inbox',
      );

      // The single-clause re-run fails late: its error must not
      // clobber the newer composed results.
      client.searchThrowAfterGate = StateError('boom');
      await _openGate(gateA);

      expect(bloc.state, isA<SearchResults>());
      expect(
        (bloc.state as SearchResults).query,
        'budgets tag:invoices in:inbox',
      );
      expect(client.searchCalls, 3);
    });

    test('a late failure of the latest re-run surfaces', () async {
      final client = FakeFfiClient();
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => bloc.state is SearchResults);

      final gateA = Completer<void>();
      final gateB = Completer<void>();
      client.searchGates
        ..add(gateA)
        ..add(gateB);

      bloc.add(
        const SearchFilterToggled(SearchFilter(key: 'tag', value: 'invoices')),
      );
      await _settle(bloc, () => client.searchCalls == 2);
      bloc.add(
        const SearchFilterToggled(SearchFilter(key: 'tag', value: 'invoices')),
      );
      await _settle(bloc, () => client.searchCalls == 3);

      client.searchThrowAfterGate = StateError('boom');
      await _openGate(gateB);
      expect(bloc.state, isA<SearchErrorState>());

      client.searchThrowAfterGate = null;
      await _openGate(gateA);

      // The older re-run resolves last but must not overwrite the
      // error state with its hits.
      expect(bloc.state, isA<SearchErrorState>());
      expect(client.searchCalls, 3);
    });

    test('a query followed by a filter toggle ends composed', () async {
      final gateA = Completer<void>();
      final gateB = Completer<void>();
      final client = FakeFfiClient();
      client.searchGates
        ..add(gateA)
        ..add(gateB);
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => client.searchCalls == 1);
      bloc.add(
        const SearchFilterToggled(SearchFilter(key: 'tag', value: 'invoices')),
      );
      await _settle(bloc, () => client.searchCalls == 2);

      // FIFO completion: the older run resolves first.
      await _openGate(gateA);
      await _openGate(gateB);

      expect(bloc.state, isA<SearchResults>());
      expect((bloc.state as SearchResults).query, 'budgets tag:invoices');
      expect(client.searchCalls, 2);
    });

    test('two filter toggles compose into one query', () async {
      final client = FakeFfiClient();
      final bloc = SearchBloc(client);
      addTearDown(bloc.close);

      bloc.add(const SearchQueryChanged('budgets'));
      await _settle(bloc, () => bloc.state is SearchResults);
      bloc.add(
        const SearchFilterToggled(SearchFilter(key: 'tag', value: 'invoices')),
      );
      await _settle(bloc, () => bloc.state is SearchResults);
      bloc.add(
        const SearchFilterToggled(SearchFilter(key: 'in', value: 'inbox')),
      );
      await _settle(bloc, () => bloc.state is SearchResults);

      expect(client.lastSearchQuery, 'budgets tag:invoices in:inbox');
      expect(
        (bloc.state as SearchResults).query,
        'budgets tag:invoices in:inbox',
      );
      expect(client.searchCalls, 3);
    });
  });
}
