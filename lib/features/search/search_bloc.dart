import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_bloc/flutter_bloc.dart';
import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/logging/log.dart';
import 'package:material_symbols_icons/symbols.dart';

/// Results of a search run, as returned by the daemon.
final class SearchResultData {
  final List<SearchHitData> hits;
  final int total;

  const SearchResultData({required this.hits, required this.total});
}

/// One message hit returned by search.
final class SearchHitData {
  final int messageId;
  final int accountId;
  final String? subject;
  final String? snippet;

  const SearchHitData({
    required this.messageId,
    required this.accountId,
    this.subject,
    this.snippet,
  });
}

/// A filter chip applied to narrow the search scope.
final class SearchFilter {
  final String
  key; // 'account', 'tag', 'is:unread', 'is:starred', 'has:attachment'
  final String value;

  const SearchFilter({required this.key, required this.value});

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is SearchFilter &&
          runtimeType == other.runtimeType &&
          key == other.key &&
          value == other.value;

  @override
  int get hashCode => Object.hash(key, value);
}

/// Inputs to [SearchBloc].
sealed class SearchEvent {
  const SearchEvent();
}

/// The user typed a new query; debounced before being sent to the daemon.
final class SearchQueryChanged extends SearchEvent {
  const SearchQueryChanged(this.query);
  final String query;
}

/// The user pressed Enter to submit the current query immediately.
final class SearchSubmitted extends SearchEvent {
  const SearchSubmitted();
}

/// The user cleared the query and filters.
final class SearchCleared extends SearchEvent {
  const SearchCleared();
}

/// The user toggled a filter chip.
final class SearchFilterToggled extends SearchEvent {
  const SearchFilterToggled(this.filter);
  final SearchFilter filter;
}

/// Outputs of [SearchBloc].
sealed class SearchState {
  const SearchState();
}

/// Idle: no query yet, show empty/recent state.
final class SearchIdle extends SearchState {
  const SearchIdle();
}

/// Loading: query has been sent, awaiting results.
final class SearchLoading extends SearchState {
  final String query;
  const SearchLoading(this.query);
}

/// Results: the daemon returned hits.
final class SearchResults extends SearchState {
  final List<SearchHitData> hits;
  final int total;
  final String query;

  const SearchResults({
    required this.hits,
    required this.total,
    required this.query,
  });
}

/// Error: the daemon rejected the query.
final class SearchErrorState extends SearchState {
  final String message;
  const SearchErrorState(this.message);
}

/// Blurs/focuses the search field; the shell wires this to Cmd/Ctrl+F.
final class SearchFocusRequested extends SearchEvent {
  const SearchFocusRequested();
}

/// Drives the search UI: debounces user input 150ms, calls
/// [FfiClient.search], and keeps the last results and active filters.
class SearchBloc extends Bloc<SearchEvent, SearchState> {
  SearchBloc(this._client) : super(const SearchIdle()) {
    on<SearchQueryChanged>(_onQueryChanged);
    on<SearchSubmitted>(_onSubmitted);
    on<SearchCleared>(_onCleared);
    on<SearchFilterToggled>(_onFilterToggled);
    on<SearchFocusRequested>(_onFocusRequested);
  }

  final FfiClient _client;
  Timer? _debounceTimer;

  /// The current filter set; persisted across queries.
  final List<SearchFilter> _filters = [];

  /// The current free-text query text.
  String _currentQueryText = '';

  @override
  Future<void> close() async {
    _debounceTimer?.cancel();
    await super.close();
  }

  /// Debounces user input by 150ms (USER_FLOW §7).
  Future<void> _onQueryChanged(
    SearchQueryChanged event,
    Emitter<SearchState> emit,
  ) async {
    _currentQueryText = event.query;
    _debounceTimer?.cancel();
    await Future.delayed(const Duration(milliseconds: 150));
    if (emit.isDone) return;
    // Only run if this is still the latest query.
    if (event.query != _currentQueryText) return;
    emit(SearchLoading(event.query));
    try {
      final result = await _client.search(
        accountId: -1,
        query: _buildQuery(event.query),
      );
      if (emit.isDone) return;
      emit(
        SearchResults(
          hits: result.messages
              .map(
                (h) => SearchHitData(
                  messageId: h.messageId.toInt(),
                  accountId: h.accountId.toInt(),
                  subject: h.subject,
                  snippet: h.snippet,
                ),
              )
              .toList(),
          total: result.total.toInt(),
          query: event.query,
        ),
      );
    } on AppFailure catch (failure) {
      if (emit.isDone) return;
      emit(SearchErrorState(failure.message));
    } catch (error, stackTrace) {
      logError('search_failed', error: error, stackTrace: stackTrace);
      if (emit.isDone) return;
      emit(const SearchErrorState('Search failed — try again.'));
    }
  }

  Future<void> _onSubmitted(
    SearchSubmitted event,
    Emitter<SearchState> emit,
  ) async {
    // Already loading from debounce; the result will arrive.
  }

  Future<void> _onCleared(
    SearchCleared event,
    Emitter<SearchState> emit,
  ) async {
    _debounceTimer?.cancel();
    _currentQueryText = '';
    _filters.clear();
    emit(const SearchIdle());
  }

  Future<void> _onFilterToggled(
    SearchFilterToggled event,
    Emitter<SearchState> emit,
  ) async {
    final existing = _filters.indexWhere((f) => f.key == event.filter.key);
    if (existing >= 0) {
      _filters.removeAt(existing);
    } else {
      _filters.add(event.filter);
    }
    final currentQuery = _currentQueryText;
    if (currentQuery.isNotEmpty) {
      emit(SearchLoading(currentQuery));
      try {
        final result = await _client.search(
          accountId: -1,
          query: _buildQuery(currentQuery),
        );
        if (emit.isDone) return;
        emit(
          SearchResults(
            hits: result.messages
                .map(
                  (h) => SearchHitData(
                    messageId: h.messageId.toInt(),
                    accountId: h.accountId.toInt(),
                    subject: h.subject,
                    snippet: h.snippet,
                  ),
                )
                .toList(),
            total: result.total.toInt(),
            query: _buildQuery(currentQuery),
          ),
        );
      } on AppFailure catch (failure) {
        if (emit.isDone) return;
        emit(SearchErrorState(failure.message));
      } catch (error, stackTrace) {
        logError('search_filter_failed', error: error, stackTrace: stackTrace);
        if (emit.isDone) return;
        emit(const SearchErrorState('Search failed — try again.'));
      }
    } else {
      emit(const SearchIdle());
    }
  }

  Future<void> _onFocusRequested(
    SearchFocusRequested event,
    Emitter<SearchState> emit,
  ) async {
    // The shell widget focuses the text field when this event arrives.
  }

  /// Builds the full query string including filter clauses.
  String _buildQuery(String freeText) {
    final parts = <String>[];
    if (freeText.isNotEmpty) parts.add(freeText);
    for (final f in _filters) {
      if (f.value.isNotEmpty) {
        parts.add('${f.key}:${f.value}');
      }
    }
    return parts.join(' ');
  }
}

/// Convenience extension to build a filter chip label and icon.
extension SearchFilterLabel on SearchFilter {
  String get label {
    switch (key) {
      case 'is:unread':
        return 'Unread';
      case 'is:starred':
        return 'Starred';
      case 'has:attachment':
        return 'Has attachment';
      case 'tag':
        return 'Tag: $value';
      case 'account':
        return 'Account: $value';
      default:
        return '$key: $value';
    }
  }

  IconData get icon {
    switch (key) {
      case 'is:unread':
        return Symbols.mark_email_read;
      case 'is:starred':
        return Symbols.star;
      case 'has:attachment':
        return Symbols.attach_file;
      case 'tag':
        return Symbols.label;
      case 'account':
        return Symbols.account_circle;
      default:
        return Symbols.filter_list;
    }
  }
}
