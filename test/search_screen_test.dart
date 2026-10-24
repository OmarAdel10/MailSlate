import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_bloc/flutter_bloc.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/core/theme/theme.dart';
import 'package:mailslate/features/search/search_bloc.dart';
import 'package:mailslate/features/search/search_screen.dart';
import 'package:mailslate/src/rust/api/search.dart' as rust_search;

import 'support/fake_ffi_client.dart';

/// SearchBloc that records every event delivered through add().
final class RecordingSearchBloc extends SearchBloc {
  RecordingSearchBloc(super.client);

  final List<SearchEvent> addedEvents = [];

  @override
  void add(SearchEvent event) {
    addedEvents.add(event);
    super.add(event);
  }
}

Future<RecordingSearchBloc> _pump(
  WidgetTester tester, {
  FakeFfiClient? client,
  VoidCallback? onClose,
}) async {
  final resolvedClient = client ?? FakeFfiClient();
  final bloc = RecordingSearchBloc(resolvedClient);
  addTearDown(bloc.close);
  await tester.pumpWidget(
    MaterialApp(
      theme: buildTheme(Brightness.light),
      home: MediaQuery(
        // Neutralizes the caret-blink timer: a focused TextField keeps
        // a repeating timer pending at test end otherwise.
        data: const MediaQueryData(
          size: Size(800, 600),
          disableAnimations: true,
        ),
        child: BlocProvider<SearchBloc>.value(
          value: bloc,
          child: SearchScreen(onClose: onClose),
        ),
      ),
    ),
  );
  return bloc;
}

/// Runs everything pending, including the 150ms debounce timer.
Future<void> _settleAll(WidgetTester tester) async {
  await tester.pump(const Duration(milliseconds: 250));
  await tester.pump(const Duration(milliseconds: 400));
}

/// Unmounts the tree before the test ends so the focused caret timer
/// cancels with its widget (flutter_test pending-timer invariant).
Future<void> _unmount(WidgetTester tester) async {
  await tester.pumpWidget(const SizedBox());
  await tester.pump();
}

void main() {
  group('SearchScreen idle', () {
    testWidgets('asks for focus and shows the placeholder', (tester) async {
      final bloc = await _pump(tester);

      expect(find.text('Search messages'), findsOneWidget);
      expect(find.byKey(const Key('search-close')), findsOneWidget);
      expect(bloc.addedEvents.whereType<SearchFocusRequested>(), hasLength(1));

      await _unmount(tester);
    });

    testWidgets('mount never sends a query to the daemon', (tester) async {
      final client = FakeFfiClient();
      await _pump(tester, client: client);

      // Even past the debounce window the daemon must never have been
      // asked anything: the IME attach happens before any typing.
      await _settleAll(tester);
      expect(client.searchCalls, 0);

      await _unmount(tester);
    });

    testWidgets('close button calls back to the shell', (tester) async {
      var closed = 0;
      await _pump(tester, onClose: () => closed += 1);

      await tester.tap(find.byKey(const Key('search-close')));
      await tester.pump();

      expect(closed, 1);

      await _unmount(tester);
    });
  });

  group('SearchScreen results', () {
    testWidgets('renders hit rows for the running query', (tester) async {
      final client = FakeFfiClient(
        searchResult: const rust_search.SearchResult(
          messages: [
            rust_search.SearchHit(
              messageId: 7,
              accountId: 42,
              subject: 'Quarterly report',
              snippet: 'the budgets look fine',
            ),
          ],
          total: 1,
        ),
      );
      await _pump(tester, client: client);

      await tester.enterText(find.byType(TextField), 'budgets');
      await _settleAll(tester);

      expect(find.text('Quarterly report'), findsOneWidget);
      expect(find.text('the budgets look fine'), findsOneWidget);
      expect(find.byKey(const Key('search-hit-7')), findsOneWidget);
      expect(find.byKey(const Key('search-clear')), findsOneWidget);

      await _unmount(tester);
    });

    testWidgets('shows the no-results lead for an empty page', (tester) async {
      await _pump(
        tester,
        client: FakeFfiClient(
          searchResult: const rust_search.SearchResult(messages: [], total: 0),
        ),
      );

      await tester.enterText(find.byType(TextField), 'ghost');
      await _settleAll(tester);

      expect(find.text('No results'), findsOneWidget);
      expect(find.text('No messages match "ghost"'), findsOneWidget);
      expect(find.byKey(const Key('search-hit-1')), findsNothing);

      await _unmount(tester);
    });

    testWidgets('shows the searching copy while results are pending', (
      tester,
    ) async {
      final gate = Completer<void>();
      await _pump(tester, client: FakeFfiClient(searchGate: gate));

      await tester.enterText(find.byType(TextField), 'budgets');
      await tester.pump(const Duration(milliseconds: 200));
      expect(find.text('Searching "budgets"…'), findsOneWidget);

      gate.complete();
      await _settleAll(tester);
      expect(find.byKey(const Key('search-hit-1')), findsOneWidget);

      await _unmount(tester);
    });

    testWidgets('clear button resets to the placeholder', (tester) async {
      await _pump(tester);

      await tester.enterText(find.byType(TextField), 'budgets');
      await _settleAll(tester);
      expect(find.text('Search messages'), findsNothing);

      await tester.tap(find.byKey(const Key('search-clear')));
      await _settleAll(tester);

      expect(find.text('Search messages'), findsOneWidget);
    });
  });

  group('SearchScreen error', () {
    testWidgets('shows the failure and lets the user retry', (tester) async {
      await _pump(
        tester,
        client: FakeFfiClient(
          searchFailure: const NetworkFailure(message: 'daemon down'),
        ),
      );

      await tester.enterText(find.byType(TextField), 'budgets');
      await _settleAll(tester);

      expect(find.text('Search failed'), findsOneWidget);
      expect(find.text('daemon down'), findsOneWidget);
      expect(find.text('Try again'), findsOneWidget);

      await _unmount(tester);
    });

    testWidgets('retry returns to the placeholder', (tester) async {
      await _pump(
        tester,
        client: FakeFfiClient(searchThrow: StateError('boom')),
      );

      await tester.enterText(find.byType(TextField), 'budgets');
      await _settleAll(tester);
      expect(find.text('Search failed'), findsOneWidget);

      await tester.tap(find.text('Try again'));
      await _settleAll(tester);

      expect(find.text('Search messages'), findsOneWidget);

      await _unmount(tester);
    });
  });
}
