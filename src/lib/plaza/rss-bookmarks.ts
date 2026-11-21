/**
 * Daily RSS bookmarks (local only). The Host appends one entry per article to
 * `notes/rss/YYYY-MM-DD.md` in the vault and keeps a rebuildable index; the
 * cloud archive never receives star, tag or vault state.
 */

import {
	commands,
	type FeedItem,
	type FeedSource,
	type RssBookmark,
	type RssBookmarkResult,
} from "@/lib/core/bindings";
import { callApi } from "@/lib/core/ipc";

export type { RssBookmark };

export const MAX_BOOKMARK_TAGS = 20;
export const MAX_BOOKMARK_TAG_CHARS = 40;

function pad(n: number): string {
	return String(n).padStart(2, "0");
}

/** Client-local day and minute the user confirmed the bookmark. */
export function localArchiveStamp(now = new Date()): {
	date: string;
	time: string;
} {
	const date = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
	return {
		date,
		time: `${date} ${pad(now.getHours())}:${pad(now.getMinutes())}`,
	};
}

export function rssDailyNotePath(date: string): string {
	return `notes/rss/${date}.md`;
}

/** Trim, collapse whitespace and drop duplicates; empty tags are skipped. */
export function normalizeBookmarkTag(raw: string): string {
	return raw.replace(/[、\s]+/g, " ").trim();
}

export async function rssDailyBookmark(args: {
	vaultPath: string;
	source: FeedSource;
	item: FeedItem;
	tags: string[];
	now?: Date;
}): Promise<RssBookmarkResult> {
	const { date, time } = localArchiveStamp(args.now);
	const { item } = args;
	return callApi(
		() =>
			commands.rssDailyBookmark({
				vaultPath: args.vaultPath,
				input: {
					source: args.source,
					articleId: item.id,
					title: item.title,
					feedTitle: item.subscriptionTitle,
					url: item.url?.trim() || item.paperUrl?.trim() || null,
					publishedAt: item.publishedAt,
					summaryText: item.summaryText,
					bodyMarkdown: item.bodyMarkdown,
					tags: args.tags,
					archiveDate: date,
					bookmarkedAt: time,
				},
			}),
		{ fallback: "feeds.bookmarkFailed" },
	);
}

export async function rssDailyStatuses(
	vaultPath: string,
	source: FeedSource,
	articleIds: string[],
): Promise<RssBookmark[]> {
	if (articleIds.length === 0) return [];
	return callApi(
		() => commands.rssDailyStatuses({ vaultPath, source, articleIds }),
		{ fallback: "feeds.bookmarkFailed" },
	);
}

export async function rssDailyList(
	vaultPath: string,
	source: FeedSource,
): Promise<RssBookmark[]> {
	return callApi(() => commands.rssDailyList({ vaultPath, source }), {
		fallback: "feeds.bookmarkFailed",
	});
}

export async function rssDailyRemove(
	vaultPath: string,
	source: FeedSource,
	articleId: string,
): Promise<void> {
	await callApi(
		() => commands.rssDailyRemove({ vaultPath, source, articleId }),
		{ fallback: "feeds.bookmarkFailed" },
	);
}

/** Row for the starred-only list; the body is resolved when opened. */
export function bookmarkToFeedItem(bookmark: RssBookmark): FeedItem {
	return {
		id: bookmark.articleId,
		subscriptionId: "",
		subscriptionTitle: bookmark.feedTitle,
		title: bookmark.title,
		url: bookmark.url,
		publishedAt: null,
		summaryText: "",
		contentHtml: null,
		paperUrl: null,
		importedAt: null,
		readAt: null,
		bodyMarkdown: null,
	};
}
