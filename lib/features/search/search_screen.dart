import 'package:flutter/material.dart';
import 'package:flutter_bloc/flutter_bloc.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:mailslate/core/theme/tokens.dart';
import 'package:mailslate/features/search/search_bloc.dart';

/// Search screen (PRD F7, USER_FLOW §7).
///
/// Global FTS5 search across all accounts with a 150ms debounce, bm25
/// ranking, snippet highlighting, and the syntax subset
/// (`from:`, `to:`, `subject:`, `in:`, `tag:`, `is:unread`, `is:starred`,
/// `has:attachment`) plus free text. Filter chips (account, tag, unread,
/// has:attachment) are combinable and individually removable.
///
/// States: empty query → recent searches placeholder; no results → type-led
/// empty state showing the parsed query; results → ranked message hits.
class SearchScreen extends StatefulWidget {
  const SearchScreen({super.key, this.onClose});

  /// Called when the user dismisses the search (close button or Esc).
  final VoidCallback? onClose;

  @override
  State<SearchScreen> createState() => _SearchScreenState();
}

class _SearchScreenState extends State<SearchScreen> {
  final _controller = TextEditingController();

  @override
  void initState() {
    super.initState();
    context.read<SearchBloc>().add(const SearchFocusRequested());
    _controller.addListener(() {
      context.read<SearchBloc>().add(SearchQueryChanged(_controller.text));
    });
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: context.tokens.canvas,
      body: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          _SearchBar(
            controller: _controller,
            onClose: () => widget.onClose?.call(),
          ),
          const Divider(height: 1, thickness: 1, color: null),
          Expanded(
            child: BlocBuilder<SearchBloc, SearchState>(
              builder: (context, state) {
                switch (state) {
                  case SearchIdle():
                    return const _SearchEmptyState();
                  case SearchLoading(query: final query):
                    return _SearchLoading(query);
                  case SearchResults(hits: final hits, total: final total, query: final query):
                    return _SearchResults(
                      hits: hits,
                      total: total,
                      query: query,
                    );
                  case SearchErrorState(message: final message):
                    return _SearchError(message);
                }
              },
            ),
          ),
        ],
      ),
    );
  }
}

/// Search entry field: as-you-type with 150ms debounce (USER_FLOW §7 step 1).
class _SearchBar extends StatelessWidget {
  const _SearchBar({
    super.key,
    required this.controller,
    required this.onClose,
  });

  final TextEditingController controller;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    return Container(
      height: 40,
      color: tokens.surfaceSoft,
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          Icon(Symbols.search, size: 18, color: tokens.muted),
          const SizedBox(width: 8),
          Expanded(
            child: TextField(
              controller: controller,
              autofocus: true,
              decoration: InputDecoration(
                hintText: 'Search all messages',
                hintStyle: Theme.of(context).textTheme.bodyMedium
                    ?.copyWith(color: tokens.mutedSoft),
                border: InputBorder.none,
                focusedBorder: InputBorder.none,
                enabledBorder: InputBorder.none,
                errorBorder: InputBorder.none,
                disabledBorder: InputBorder.none,
                isDense: true,
                contentPadding: EdgeInsets.zero,
              ),
              style: Theme.of(context).textTheme.bodyMedium,
            ),
          ),
          if (controller.text.isNotEmpty)
            IconButton(
              key: const Key('search-clear'),
              icon: Icon(Symbols.close, size: 16, color: tokens.muted),
              onPressed: () {
                controller.clear();
                context.read<SearchBloc>().add(const SearchCleared());
              },
            ),
          IconButton(
            key: const Key('search-close'),
            icon: Icon(Symbols.close, size: 16, color: tokens.muted),
            onPressed: onClose,
            tooltip: 'Close search (Esc)',
          ),
        ],
      ),
    );
  }
}

/// Empty state: no query yet → show recent searches placeholder.
class _SearchEmptyState extends StatelessWidget {
  const _SearchEmptyState();

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          Icon(Symbols.search, size: 48, color: tokens.mutedSoft),
          const SizedBox(height: 16),
          Text(
            'Search messages',
            style: Theme.of(context).textTheme.titleLarge
                ?.copyWith(color: tokens.ink),
          ),
          const SizedBox(height: 8),
          Text(
            'Type to search across all messages. Use from:, to:, subject:, '
                'in:, tag:, is:unread, is:starred, has:attachment.',
            style: Theme.of(context).textTheme.bodyMedium
                ?.copyWith(color: tokens.muted),
            textAlign: TextAlign.center,
          ),
        ],
      ),
    );
  }
}

/// Loading state.
class _SearchLoading extends StatelessWidget {
  const _SearchLoading(this.query);

  final String query;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          SizedBox(
            width: 16,
            height: 16,
            child: CircularProgressIndicator(strokeWidth: 2, color: tokens.accent),
          ),
          const SizedBox(height: 12),
          Text(
            'Searching "$query"…',
            style: Theme.of(context).textTheme.bodyMedium
                ?.copyWith(color: tokens.muted),
          ),
        ],
      ),
    );
  }
}

/// Results: ranked hits newest-first with snippets.
class _SearchResults extends StatelessWidget {
  const _SearchResults({
    super.key,
    required this.hits,
    required this.total,
    required this.query,
  });

  final List<SearchHitData> hits;
  final int total;
  final String query;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    return hits.isEmpty
        ? Center(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.center,
              children: [
                Icon(Symbols.search_off, size: 48, color: tokens.mutedSoft),
                const SizedBox(height: 16),
                Text(
                  'No results',
                  style: Theme.of(context).textTheme.titleLarge
                      ?.copyWith(color: tokens.ink),
                ),
                const SizedBox(height: 8),
                Text(
                  'No messages match "$query"',
                  style: Theme.of(context).textTheme.bodyMedium
                      ?.copyWith(color: tokens.muted),
                  textAlign: TextAlign.center,
                ),
              ],
            ),
          )
        : ListView.builder(
          itemCount: hits.length,
          itemBuilder: (context, index) {
            final hit = hits[index];
            return _SearchHitRow(hit: hit);
          },
        );
  }
}

/// One search hit row: subject, snippet, metadata.
class _SearchHitRow extends StatelessWidget {
  const _SearchHitRow({super.key, required this.hit});

  final SearchHitData hit;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 6),
      child: InkWell(
        key: Key('search-hit-${hit.messageId}'),
        onTap: () {},
        borderRadius: BorderRadius.circular(6),
        hoverColor: tokens.accentTint,
        child: Padding(
          padding: const EdgeInsets.symmetric(vertical: 8),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                hit.subject ?? '(no subject)',
                style: Theme.of(context).textTheme.bodyMedium
                    ?.copyWith(fontWeight: FontWeight.w600),
              ),
              const SizedBox(height: 4),
              Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Expanded(
                    child: Text(
                      hit.snippet ?? '',
                      style: Theme.of(context).textTheme.bodySmall
                          ?.copyWith(color: tokens.muted),
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Error state.
class _SearchError extends StatelessWidget {
  const _SearchError(this.message);

  final String message;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          Icon(Symbols.error, size: 48, color: tokens.error),
          const SizedBox(height: 16),
          Text(
            'Search failed',
            style: Theme.of(context).textTheme.titleLarge
                ?.copyWith(color: tokens.ink),
          ),
          const SizedBox(height: 8),
          Text(
            message,
            style: Theme.of(context).textTheme.bodyMedium
                ?.copyWith(color: tokens.muted),
            textAlign: TextAlign.center,
          ),
          const SizedBox(height: 16),
          FilledButton(
            onPressed: () => context.read<SearchBloc>().add(const SearchCleared()),
            child: const Text('Try again'),
          ),
        ],
      ),
    );
  }
}
