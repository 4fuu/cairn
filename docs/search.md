# Search

`cairn search` is built for recall first, then ranking. It is literal search, not semantic search.

## Three layers

1. **Trigram** (`messages_trigram`): exact substrings of three or more characters. This is what makes code identifiers, paths, error text, and Chinese searchable.
2. **Words** (`messages_fts`): jieba search-mode tokens over `messages.text_words`, indexed with FTS5 `unicode61`. Good for English words and two-character Chinese terms.
3. **LIKE**: fallback for one- or two-character queries, and when the FTS layers return nothing.

The trigram layer runs first and ranks above word matches. BM25 scores, per-session hit counts, and recency refine the order.

## What is searched

By default, only normal text messages are searched:

- user and assistant text
- `content_type = 'text'`
- `is_meta = 0`

`--include-meta` removes those filters and includes thinking, tool results, and tool-use rows. Use it when the evidence is a command output, a stack trace, or a tool call.

Tool-call **arguments** are stored in `tool_calls.input_json` but are not part of the full-text index. A command such as `pytest tests/billing` is searchable only if the command or its output also appears in a message body. The same applies to file paths that exist only inside a tool input.

## English

English works through both the trigram and word layers:

- case-insensitive for ASCII (`redirect` and `REDIRECT` match)
- substring matching (`round` matches `rounding`)
- multi-word queries are ANDed across the word index, so word order does not matter
- code identifiers such as `Date.now` and `test_invoice.py` are searchable

There is no stemming or synonym expansion. `redirect` matches `redirects` because it is a substring; `car` does not match `automobile`.

## Chinese

Jieba search-mode tokens cover common two-character words. Three or more characters also use the trigram layer, which handles longer phrases and code-mixed text.

## Result modes

- Default: group by session. Each result has the best hit, a `hits` count, a snippet, and a score.
- `--all`: return individual message hits.
- `--context N`: attach up to N neighboring messages to the best grouped hit.
- `--full-text`: return the complete matched message instead of a 280-character snippet.
- `--json`: return plain text for agents; terminal output highlights exact matches when stdout is a TTY.

## Misses

If a query returns nothing:

1. Check `cairn sources`; the provider root may not exist.
2. Try `--include-meta` for thinking, tool results, or tool calls.
3. Check whether the term exists only in a tool-call argument.
4. Use `cairn sql` to inspect `messages` or `tool_calls` directly.
5. Try a shorter substring of three or more characters.

One- and two-character queries scan `messages.text` with `LIKE`; they work but are slower on a large index.
