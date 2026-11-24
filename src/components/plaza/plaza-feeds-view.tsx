/**
 * Native 广场 panel: user RSS / Atom subscriptions and a paper-aware timeline.
 */

import { useVirtualizer } from "@tanstack/react-virtual";
import type { TFunction } from "i18next";
import {
	CheckCheck,
	ChevronRight,
	Cloud,
	EllipsisVertical,
	EyeOff,
	Inbox,
	Pin,
	Plus,
	RefreshCw,
	Rss,
	Star,
} from "lucide-react";
import {
	Fragment,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { useTranslation } from "react-i18next";
import { PlazaFeedBookmarkDialog } from "@/components/plaza/plaza-feeds-bookmark-dialog";
import {
	PlazaFeedItemDetail,
	PlazaFeedItemRow,
} from "@/components/plaza/plaza-feeds-item";
import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import {
	FloatingMenu,
	FloatingMenuItem,
	FloatingMenuSeparator,
} from "@/components/ui/floating-menu";
import { Input } from "@/components/ui/input";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import { useVaultStore } from "@/hooks/use-app-stores";
import { copyTextToClipboard } from "@/lib/core/clipboard";
import { errorText } from "@/lib/core/error";
import { notifyError, notifySuccess } from "@/lib/core/notify";
import { cn } from "@/lib/core/utils";
import {
	ARXIV_FEED_CHIPS,
	arxivFeedUrl,
	type CloudFeedSettings,
	compareFeedSubs,
	type FeedItem,
	type FeedSource,
	type FeedSub,
	feedsAdd,
	feedsCloudItem,
	feedsCloudItems,
	feedsCloudList,
	feedsCloudSetRead,
	feedsCloudSettingsGet,
	feedsCloudSettingsSet,
	feedsCloudTest,
	feedsItems,
	feedsList,
	feedsMarkAllRead,
	feedsMarkImported,
	feedsRefresh,
	feedsRemove,
	feedsRename,
	feedsResolveBody,
	feedsSetPinned,
	feedsSetRead,
	groupFeedSubs,
	hostErrorKey,
	importFeedItem,
} from "@/lib/plaza/feeds";
import {
	subscribeFeedDetail,
	takePendingFeedDetailId,
} from "@/lib/plaza/mention-open";
import {
	bookmarkToFeedItem,
	maybeRssDailyAnalyze,
	type RssBookmark,
	rssDailyBookmark,
	rssDailyList,
	rssDailyRemove,
	rssDailyStatuses,
} from "@/lib/plaza/rss-bookmarks";
import { openVaultRel } from "@/lib/workspace/viewer-actions";

const STALE_REFRESH_MS = 15 * 60 * 1000;

/** Below this width the reading pane replaces the list instead of sitting beside it. */
const READING_PANE_MIN_WIDTH = 1000;

function toastHostError(message: string, t: TFunction<"sidebar">): void {
	const key = hostErrorKey(message);
	notifyError(key ? t(key) : message);
}

function formatHostError(message: string, t: TFunction<"sidebar">): string {
	const key = hostErrorKey(message);
	return key ? t(key) : message;
}

type FeedSubMenuState = { sub: FeedSub; x: number; y: number };

function openFeedSubMenu(
	sub: FeedSub,
	event: {
		preventDefault(): void;
		stopPropagation(): void;
		clientX: number;
		clientY: number;
	},
	setMenu: (next: FeedSubMenuState) => void,
): void {
	event.preventDefault();
	event.stopPropagation();
	setMenu({ sub, x: event.clientX, y: event.clientY });
}

const HIDE_READ_KEY = "agentero-plaza-feeds-hide-read";

/** "Hide read" defaults on; the last choice is kept across restarts. */
function readHideReadPreference(): boolean {
	try {
		return window.localStorage.getItem(HIDE_READ_KEY) !== "0";
	} catch {
		return true;
	}
}

function writeHideReadPreference(hide: boolean): void {
	try {
		window.localStorage.setItem(HIDE_READ_KEY, hide ? "1" : "0");
	} catch {
		/* preference is best-effort */
	}
}

function UnreadCount({ value }: { value: number | null }) {
	if (!value) return null;
	return (
		<span className="shrink-0 text-muted-foreground text-xs tabular-nums">
			{value > 999 ? "999+" : value}
		</span>
	);
}

function FeedSubMenu({
	menu,
	cloud,
	onClose,
	onMarkRead,
	onRename,
	onCopyUrl,
	onRefresh,
	onPin,
	onRemove,
}: {
	menu: FeedSubMenuState;
	/** Cloud subscriptions are read-only: only read state and URL apply. */
	cloud: boolean;
	onClose: () => void;
	onMarkRead: (sub: FeedSub) => void;
	onRename: (sub: FeedSub) => void;
	onCopyUrl: (sub: FeedSub) => void;
	onRefresh: (sub: FeedSub) => void;
	onPin: (sub: FeedSub) => void;
	onRemove: (sub: FeedSub) => void;
}) {
	const { t } = useTranslation("sidebar");
	const menuRef = useRef<HTMLDivElement>(null);

	// Re-bind when the menu opens at a new point so the opening click
	// does not immediately dismiss it (same pattern as the file tree).
	// biome-ignore lint/correctness/useExhaustiveDependencies: intentional
	useEffect(() => {
		const onKey = (event: KeyboardEvent) => {
			if (event.key === "Escape") onClose();
		};
		const onPointer = (event: PointerEvent) => {
			const el = menuRef.current;
			if (el && event.target instanceof Node && el.contains(event.target)) {
				return;
			}
			onClose();
		};
		const timer = window.setTimeout(() => {
			window.addEventListener("pointerdown", onPointer, true);
			window.addEventListener("keydown", onKey, true);
			window.addEventListener("scroll", onClose, true);
			window.addEventListener("resize", onClose);
		}, 0);
		return () => {
			window.clearTimeout(timer);
			window.removeEventListener("pointerdown", onPointer, true);
			window.removeEventListener("keydown", onKey, true);
			window.removeEventListener("scroll", onClose, true);
			window.removeEventListener("resize", onClose);
		};
	}, [menu, onClose]);

	const run = (action: (sub: FeedSub) => void) => {
		action(menu.sub);
		onClose();
	};

	return (
		<FloatingMenu
			floatingRef={menuRef}
			point={{ x: menu.x, y: menu.y }}
			className="z-[2000] min-w-44"
		>
			<FloatingMenuItem onClick={() => run(onMarkRead)}>
				{t("plaza.feeds.markAllRead")}
			</FloatingMenuItem>
			<FloatingMenuSeparator />
			{cloud ? null : (
				<FloatingMenuItem onClick={() => run(onRename)}>
					{t("plaza.feeds.rename")}
				</FloatingMenuItem>
			)}
			<FloatingMenuItem onClick={() => run(onCopyUrl)}>
				{t("plaza.feeds.copyUrl")}
			</FloatingMenuItem>
			{cloud ? null : (
				<>
					<FloatingMenuItem onClick={() => run(onRefresh)}>
						{t("plaza.feeds.refreshOne")}
					</FloatingMenuItem>
					<FloatingMenuItem onClick={() => run(onPin)}>
						{menu.sub.pinned ? t("plaza.feeds.unpin") : t("plaza.feeds.pin")}
					</FloatingMenuItem>
					<FloatingMenuSeparator />
					<FloatingMenuItem variant="destructive" onClick={() => run(onRemove)}>
						{t("plaza.feeds.remove")}
					</FloatingMenuItem>
				</>
			)}
		</FloatingMenu>
	);
}

function AddForm({
	busy,
	refreshing,
	onAdd,
	onRefresh,
	showRefresh,
	showChips,
}: {
	busy: boolean;
	refreshing: boolean;
	onAdd: (url: string) => void;
	onRefresh: () => void;
	showRefresh: boolean;
	showChips: boolean;
}) {
	const { t } = useTranslation("sidebar");
	const [url, setUrl] = useState("");
	return (
		<form
			className="space-y-2"
			onSubmit={(event) => {
				event.preventDefault();
				const next = url.trim();
				if (!next || busy) return;
				onAdd(next);
				setUrl("");
			}}
		>
			<div className="flex items-center gap-1.5">
				<Input
					value={url}
					onChange={(event) => setUrl(event.target.value)}
					placeholder={t("plaza.feeds.urlPlaceholder")}
					disabled={busy}
					aria-label={t("plaza.feeds.urlPlaceholder")}
					className="h-8 min-w-0 flex-1"
				/>
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							type="submit"
							size="icon"
							disabled={busy || !url.trim()}
							aria-label={t("plaza.feeds.add")}
						>
							<Plus className="size-3.5" aria-hidden />
						</Button>
					</TooltipTrigger>
					<TooltipContent>{t("plaza.feeds.add")}</TooltipContent>
				</Tooltip>
				{showRefresh ? (
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								type="button"
								variant="ghost"
								size="icon"
								disabled={refreshing}
								aria-label={t("plaza.feeds.refresh")}
								onClick={onRefresh}
							>
								<RefreshCw
									className={cn("size-3.5", refreshing && "animate-spin")}
									aria-hidden
								/>
							</Button>
						</TooltipTrigger>
						<TooltipContent>{t("plaza.feeds.refresh")}</TooltipContent>
					</Tooltip>
				) : null}
			</div>
			{showChips ? (
				<div className="flex flex-wrap gap-1">
					{ARXIV_FEED_CHIPS.map((cat) => (
						<Button
							key={cat}
							type="button"
							variant="outline"
							size="xs"
							disabled={busy}
							onClick={() => onAdd(arxivFeedUrl(cat))}
						>
							{cat}
						</Button>
					))}
				</div>
			) : null}
		</form>
	);
}

function CloudSourceDialog({
	open,
	settings,
	onOpenChange,
	onSaved,
}: {
	open: boolean;
	settings: CloudFeedSettings | null;
	onOpenChange: (open: boolean) => void;
	onSaved: (next: CloudFeedSettings) => void;
}) {
	const { t } = useTranslation("sidebar");
	const [source, setSource] = useState<FeedSource>("local");
	const [baseUrl, setBaseUrl] = useState("");
	const [token, setToken] = useState("");
	const [busy, setBusy] = useState(false);

	useEffect(() => {
		if (!open) return;
		setSource(settings?.source ?? "local");
		setBaseUrl(settings?.baseUrl ?? "");
		setToken("");
	}, [open, settings]);

	const run = async (action: () => Promise<void>) => {
		setBusy(true);
		try {
			await action();
		} catch (error) {
			toastHostError(errorText(error), t);
		} finally {
			setBusy(false);
		}
	};

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-sm">
				<DialogHeader>
					<DialogTitle>{t("plaza.feeds.cloudSettings")}</DialogTitle>
				</DialogHeader>
				<div className="space-y-3">
					<div
						role="radiogroup"
						aria-label={t("plaza.feeds.source")}
						className="grid grid-cols-2 gap-1 rounded-lg bg-muted p-0.5"
					>
						{(["local", "cloud"] as const).map((value) => (
							<Button
								key={value}
								type="button"
								role="radio"
								aria-checked={source === value}
								variant="ghost"
								size="sm"
								className={cn(source === value && "bg-background shadow-sm")}
								onClick={() => setSource(value)}
							>
								{t(
									value === "local"
										? "plaza.feeds.sourceLocal"
										: "plaza.feeds.sourceCloud",
								)}
							</Button>
						))}
					</div>
					<Input
						value={baseUrl}
						onChange={(event) => setBaseUrl(event.target.value)}
						placeholder="https://rss.example.org"
						aria-label={t("plaza.feeds.cloudUrl")}
						disabled={busy}
					/>
					<Input
						type="password"
						autoComplete="off"
						value={token}
						onChange={(event) => setToken(event.target.value)}
						placeholder={
							settings?.hasToken
								? t("plaza.feeds.cloudTokenSaved")
								: t("plaza.feeds.cloudToken")
						}
						aria-label={t("plaza.feeds.cloudToken")}
						disabled={busy}
					/>
					<p className="text-muted-foreground text-xs leading-relaxed">
						{t("plaza.feeds.cloudHint")}
					</p>
				</div>
				<DialogFooter>
					<Button
						type="button"
						variant="ghost"
						disabled={busy || !baseUrl.trim()}
						onClick={() =>
							void run(async () => {
								await feedsCloudTest(baseUrl, token.trim() || undefined);
								notifySuccess(t("plaza.feeds.cloudConnected"));
							})
						}
					>
						{t("plaza.feeds.cloudTest")}
					</Button>
					<Button
						type="button"
						disabled={busy}
						onClick={() =>
							void run(async () => {
								const next = await feedsCloudSettingsSet({
									source,
									baseUrl,
									token: token.trim() || undefined,
								});
								onSaved(next);
								onOpenChange(false);
							})
						}
					>
						{t("plaza.feeds.save")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

export function PlazaFeedsView({ className }: { className?: string }) {
	const { t } = useTranslation("sidebar");
	const [subs, setSubs] = useState<FeedSub[]>([]);
	const subsRef = useRef(subs);
	subsRef.current = subs;
	const [hideRead, setHideRead] = useState(readHideReadPreference);
	const hideReadRef = useRef(hideRead);
	hideReadRef.current = hideRead;
	const itemsRequest = useRef(0);
	const importPending = useRef(new Set<string>());
	const [importingIds, setImportingIds] = useState<Set<string>>(
		() => new Set(),
	);
	const readPending = useRef(new Set<string>());
	const [items, setItems] = useState<FeedItem[]>([]);
	const itemsRef = useRef(items);
	itemsRef.current = items;
	const [selectedId, setSelectedId] = useState<string | null>(null);
	const [loading, setLoading] = useState(true);
	const [busy, setBusy] = useState(false);
	const [refreshing, setRefreshing] = useState(false);
	const [markingAll, setMarkingAll] = useState(false);
	const [collapsedFolders, setCollapsedFolders] = useState<Set<string>>(
		() => new Set(),
	);
	const [renameTarget, setRenameTarget] = useState<FeedSub | null>(null);
	const [renameTitle, setRenameTitle] = useState("");
	const [openItem, setOpenItem] = useState<FeedItem | null>(null);
	const [subMenu, setSubMenu] = useState<FeedSubMenuState | null>(null);
	const listRef = useRef<HTMLDivElement>(null);
	const closeSubMenu = useCallback(() => setSubMenu(null), []);
	const [cloudSettings, setCloudSettings] = useState<CloudFeedSettings | null>(
		null,
	);
	const [cloudDialogOpen, setCloudDialogOpen] = useState(false);
	const source: FeedSource | null = cloudSettings?.source ?? null;
	const cloud = source === "cloud";
	const cloudRef = useRef(cloud);
	cloudRef.current = cloud;
	const nextCursorRef = useRef<string | null>(null);
	const [hasMore, setHasMore] = useState(false);
	const loadingMore = useRef(false);
	const vaultPath = useVaultStore((s) => s.vaultPath);
	const vaultPathRef = useRef(vaultPath);
	vaultPathRef.current = vaultPath;
	const [starredOnly, setStarredOnly] = useState(false);
	const starredOnlyRef = useRef(starredOnly);
	starredOnlyRef.current = starredOnly;
	const [bookmarks, setBookmarks] = useState<Map<string, RssBookmark>>(
		() => new Map(),
	);
	const [bookmarkTarget, setBookmarkTarget] = useState<FeedItem | null>(null);
	const [panesEl, setPanesEl] = useState<HTMLDivElement | null>(null);
	const [widePanes, setWidePanes] = useState(false);

	useEffect(() => {
		if (!panesEl) return;
		const update = () =>
			setWidePanes(panesEl.clientWidth >= READING_PANE_MIN_WIDTH);
		update();
		const observer = new ResizeObserver(update);
		observer.observe(panesEl);
		return () => observer.disconnect();
	}, [panesEl]);

	useEffect(() => {
		void feedsCloudSettingsGet()
			.then(setCloudSettings)
			.catch(() =>
				setCloudSettings({ source: "local", baseUrl: "", hasToken: false }),
			);
	}, []);

	const loadItems = useCallback(
		async (subscriptionId: string | null, closeDetail = true) => {
			const request = ++itemsRequest.current;
			if (closeDetail) setOpenItem(null);
			try {
				if (starredOnlyRef.current) {
					nextCursorRef.current = null;
					setHasMore(false);
					const vault = vaultPathRef.current;
					const rows = vault
						? await rssDailyList(vault, cloudRef.current ? "cloud" : "local")
						: [];
					if (request !== itemsRequest.current) return;
					const subTitle = subscriptionId
						? subsRef.current.find((row) => row.id === subscriptionId)?.title
						: undefined;
					setBookmarks(new Map(rows.map((row) => [row.articleId, row])));
					setItems(
						rows
							.filter((row) => !subTitle || row.feedTitle === subTitle)
							.map(bookmarkToFeedItem),
					);
					return;
				}
				if (cloudRef.current) {
					const page = await feedsCloudItems({
						feedId: subscriptionId ?? undefined,
						unreadOnly: hideReadRef.current,
					});
					if (request !== itemsRequest.current) return;
					nextCursorRef.current = page.nextCursor;
					setHasMore(page.nextCursor !== null);
					setItems(page.items);
					return;
				}
				nextCursorRef.current = null;
				setHasMore(false);
				const rows = await feedsItems({
					subscriptionId: subscriptionId ?? undefined,
					filter: hideReadRef.current ? "unread_session" : "all",
				});
				if (request === itemsRequest.current) setItems(rows);
			} catch (error) {
				toastHostError(errorText(error), t);
			}
		},
		[t],
	);

	const selectedIdRef = useRef(selectedId);
	selectedIdRef.current = selectedId;

	/** Next cloud page by cursor; appends, never refetches what is loaded. */
	const loadMore = useCallback(async () => {
		const cursor = nextCursorRef.current;
		if (!cloudRef.current || !cursor || loadingMore.current) return;
		loadingMore.current = true;
		const request = itemsRequest.current;
		try {
			const page = await feedsCloudItems({
				feedId: selectedIdRef.current ?? undefined,
				cursor,
				unreadOnly: hideReadRef.current,
			});
			if (request !== itemsRequest.current) return;
			nextCursorRef.current = page.nextCursor;
			setHasMore(page.nextCursor !== null);
			setItems((prev) => {
				const seen = new Set(prev.map((row) => row.id));
				return [...prev, ...page.items.filter((row) => !seen.has(row.id))];
			});
		} catch (error) {
			toastHostError(errorText(error), t);
		} finally {
			loadingMore.current = false;
		}
	}, [t]);

	useEffect(() => {
		if (source === null) return;
		let cancelled = false;
		void (async () => {
			setLoading(true);
			itemsRequest.current++;
			setOpenItem(null);
			setSubs([]);
			setItems([]);
			setSelectedId(null);
			selectedIdRef.current = null;
			try {
				const next =
					source === "cloud" ? await feedsCloudList() : await feedsList();
				if (cancelled) return;
				setSubs([...next].sort(compareFeedSubs));
				await loadItems(selectedIdRef.current, false);
			} catch (error) {
				if (!cancelled) {
					toastHostError(errorText(error), t);
				}
			} finally {
				if (!cancelled) setLoading(false);
			}
		})();
		return () => {
			cancelled = true;
		};
	}, [loadItems, source, t]);

	useEffect(() => {
		// The cloud server polls on its own; never touch local feeds there.
		if (source !== "local") return;
		let cancelled = false;
		const runStale = async () => {
			try {
				const result = await feedsRefresh({ staleOnly: true });
				if (cancelled) return;
				setSubs([...result.subscriptions].sort(compareFeedSubs));
				await loadItems(selectedIdRef.current, false);
			} catch {
				/* stale refresh is best-effort */
			}
		};
		void runStale();
		const timer = window.setInterval(() => void runStale(), STALE_REFRESH_MS);
		return () => {
			cancelled = true;
			window.clearInterval(timer);
		};
	}, [loadItems, source]);

	const onAdd = useCallback(
		async (url: string) => {
			setBusy(true);
			try {
				const sub = await feedsAdd(url);
				setSubs((prev) =>
					prev.some((row) => row.id === sub.id)
						? prev
						: [...prev, sub].sort(compareFeedSubs),
				);
				setSelectedId(sub.id);
				await loadItems(sub.id);
			} catch (error) {
				toastHostError(errorText(error), t);
			} finally {
				setBusy(false);
			}
		},
		[loadItems, t],
	);

	const onRefresh = useCallback(async () => {
		setRefreshing(true);
		try {
			if (cloudRef.current) {
				setSubs(await feedsCloudList());
				await loadItems(selectedId);
				return;
			}
			const result = await feedsRefresh({
				id: selectedId ?? undefined,
			});
			setSubs([...result.subscriptions].sort(compareFeedSubs));
			await loadItems(selectedId);
		} catch (error) {
			toastHostError(errorText(error), t);
		} finally {
			setRefreshing(false);
		}
	}, [loadItems, selectedId, t]);

	const onRemove = useCallback(
		async (id: string) => {
			try {
				await feedsRemove(id);
				setSubs((prev) => prev.filter((row) => row.id !== id));
				if (selectedId === id) {
					setSelectedId(null);
					await loadItems(null);
				} else {
					await loadItems(selectedId);
				}
			} catch (error) {
				toastHostError(errorText(error), t);
			}
		},
		[loadItems, selectedId, t],
	);

	const applySub = useCallback((next: FeedSub) => {
		setSubs((prev) =>
			prev
				.map((row) => (row.id === next.id ? next : row))
				.sort(compareFeedSubs),
		);
	}, []);

	const applyItem = useCallback((next: FeedItem) => {
		setItems((prev) =>
			prev.map((row) =>
				row.id === next.id ? { ...next, readAt: row.readAt } : row,
			),
		);
		setOpenItem((current) =>
			current?.id === next.id ? { ...next, readAt: current.readAt } : current,
		);
	}, []);

	const importItem = useCallback(
		async (item: FeedItem) => {
			if (item.importedAt || importPending.current.has(item.id)) return;
			importPending.current.add(item.id);
			setImportingIds(new Set(importPending.current));
			try {
				if (await importFeedItem(item)) {
					applyItem(
						cloudRef.current
							? { ...item, importedAt: new Date().toISOString() }
							: await feedsMarkImported(item.id),
					);
				}
			} catch (error) {
				toastHostError(errorText(error), t);
			} finally {
				importPending.current.delete(item.id);
				setImportingIds(new Set(importPending.current));
			}
		},
		[applyItem, t],
	);

	const setRead = useCallback(
		async (item: FeedItem, read: boolean) => {
			if (readPending.current.has(item.id)) return;
			readPending.current.add(item.id);
			try {
				const next = await (cloudRef.current
					? feedsCloudSetRead(item.id, read)
					: feedsSetRead(item.id, read));
				if (!item.readAt !== !next.readAt) {
					setSubs((rows) =>
						rows.map((row) =>
							row.id === item.subscriptionId && row.unreadCount !== null
								? {
										...row,
										unreadCount: Math.max(
											0,
											row.unreadCount + (next.readAt ? -1 : 1),
										),
									}
								: row,
						),
					);
				}
				setItems((rows) =>
					rows.map((row) =>
						row.id === item.id ? { ...row, readAt: next.readAt } : row,
					),
				);
				setOpenItem((current) =>
					current?.id === item.id
						? { ...current, readAt: next.readAt }
						: current,
				);
			} catch (error) {
				toastHostError(errorText(error), t);
			} finally {
				readPending.current.delete(item.id);
			}
		},
		[t],
	);

	const onMarkAllRead = useCallback(
		async (feedId: string | null = selectedId) => {
			setMarkingAll(true);
			try {
				setSubs(
					[...(await feedsMarkAllRead(cloudRef.current, feedId))].sort(
						compareFeedSubs,
					),
				);
				await loadItems(selectedId, false);
			} catch (error) {
				toastHostError(errorText(error), t);
			} finally {
				setMarkingAll(false);
			}
		},
		[loadItems, selectedId, t],
	);

	const openFeedItem = useCallback(
		(item: FeedItem) => {
			setOpenItem(item);
			if (!item.readAt) void setRead(item, true);
		},
		[setRead],
	);

	/**
	 * Chip click in the Agent composer: reopen this item's detail. The list
	 * usually holds it; an older chip falls back to one body resolve, which is
	 * what opening the detail would do anyway. Failures leave the list alone.
	 */
	const openFeedById = useCallback(
		async (feedId: string) => {
			const local = itemsRef.current.find((row) => row.id === feedId);
			if (local) {
				openFeedItem(local);
				return;
			}
			try {
				openFeedItem(
					await (cloudRef.current
						? feedsCloudItem(feedId)
						: feedsResolveBody(feedId)),
				);
			} catch {
				// Opening is best-effort; the Plaza tab still shows the list.
			}
		},
		[openFeedItem],
	);

	useEffect(() => {
		const pending = takePendingFeedDetailId();
		if (pending) void openFeedById(pending);
		return subscribeFeedDetail((feedId) => void openFeedById(feedId));
	}, [openFeedById]);

	const onPin = useCallback(
		async (id: string, pinned: boolean) => {
			try {
				applySub(await feedsSetPinned(id, pinned));
			} catch (error) {
				toastHostError(errorText(error), t);
			}
		},
		[applySub, t],
	);

	const onRename = useCallback(async () => {
		if (!renameTarget) return;
		const title = renameTitle.trim();
		if (!title) return;
		try {
			const next = await feedsRename(renameTarget.id, title);
			applySub(next);
			setItems((prev) =>
				prev.map((row) =>
					row.subscriptionId === next.id
						? { ...row, subscriptionTitle: next.title }
						: row,
				),
			);
			setRenameTarget(null);
		} catch (error) {
			toastHostError(errorText(error), t);
		}
	}, [applySub, renameTarget, renameTitle, t]);

	useEffect(() => {
		if (source === null || starredOnly) return;
		if (!vaultPath) {
			setBookmarks(new Map());
			return;
		}
		let cancelled = false;
		const ids = items.map((row) => row.id);
		void rssDailyStatuses(vaultPath, source, ids)
			.then((rows) => {
				if (cancelled) return;
				setBookmarks(new Map(rows.map((row) => [row.articleId, row])));
			})
			.catch(() => {
				/* star state is best-effort; bookmarking reports its own errors */
			});
		return () => {
			cancelled = true;
		};
	}, [items, source, starredOnly, vaultPath]);

	const onStar = useCallback(
		(item: FeedItem) => {
			if (!vaultPathRef.current) {
				notifyError(t("plaza.feeds.errors.bookmark_no_vault"));
				return;
			}
			setBookmarkTarget(item);
		},
		[t],
	);

	const confirmBookmark = useCallback(
		async (item: FeedItem, tags: string[]) => {
			const vault = vaultPathRef.current;
			if (!vault || source === null) {
				notifyError(t("plaza.feeds.errors.bookmark_no_vault"));
				return;
			}
			let full = item;
			if (!full.bodyMarkdown?.trim()) {
				try {
					full = await (cloudRef.current
						? feedsCloudItem(item.id)
						: feedsResolveBody(item.id));
				} catch {
					/* fall back to the feed excerpt */
				}
			}
			try {
				const result = await rssDailyBookmark({
					vaultPath: vault,
					source,
					item: {
						...full,
						subscriptionTitle: item.subscriptionTitle || full.subscriptionTitle,
					},
					tags,
				});
				setBookmarks((prev) => new Map(prev).set(item.id, result.bookmark));
				setBookmarkTarget(null);
				if (result.bookmark.aiStatus === "not_started") {
					void maybeRssDailyAnalyze({
						vaultPath: vault,
						source,
						item: full,
						bookmark: result.bookmark,
					}).catch((error) => {
						notifyError(errorText(error) || t("plaza.feeds.analysisFailed"));
					});
				}
				if (result.created) {
					notifySuccess(
						t("plaza.feeds.bookmarkSaved", { path: result.bookmark.notePath }),
					);
				} else {
					openVaultRel(result.bookmark.notePath);
				}
			} catch (error) {
				toastHostError(errorText(error), t);
			}
		},
		[source, t],
	);

	const removeBookmark = useCallback(
		async (bookmark: RssBookmark) => {
			const vault = vaultPathRef.current;
			if (!vault || source === null) return;
			try {
				await rssDailyRemove(vault, source, bookmark.articleId);
				setBookmarks((prev) => {
					const next = new Map(prev);
					next.delete(bookmark.articleId);
					return next;
				});
				setBookmarkTarget(null);
				if (starredOnlyRef.current) {
					setItems((prev) =>
						prev.filter((row) => row.id !== bookmark.articleId),
					);
				}
			} catch (error) {
				toastHostError(errorText(error), t);
			}
		},
		[source, t],
	);

	const virtualizer = useVirtualizer({
		count: items.length,
		getScrollElement: () => listRef.current,
		estimateSize: () => 92,
		overscan: 8,
	});

	const virtualRows = virtualizer.getVirtualItems();
	const lastVirtualIndex = virtualRows[virtualRows.length - 1]?.index ?? -1;
	useEffect(() => {
		if (hasMore && lastVirtualIndex >= items.length - 8) {
			void loadMore();
		}
	}, [hasMore, lastVirtualIndex, loadMore, items.length]);

	const cloudButton = (
		<Tooltip>
			<TooltipTrigger asChild>
				<Button
					type="button"
					variant="ghost"
					size="icon-sm"
					aria-label={t("plaza.feeds.cloudSettings")}
					aria-pressed={cloud}
					className={cn(
						"shrink-0 bg-foreground/5 hover:bg-foreground/10",
						cloud && "bg-foreground/10 text-foreground",
					)}
					onClick={() => setCloudDialogOpen(true)}
				>
					<Cloud className="size-3.5" aria-hidden />
				</Button>
			</TooltipTrigger>
			<TooltipContent>{t("plaza.feeds.cloudSettings")}</TooltipContent>
		</Tooltip>
	);

	const selected = selectedId
		? (subs.find((row) => row.id === selectedId) ?? null)
		: null;
	const emptyItems = !loading && items.length === 0;
	const subGroups = useMemo(() => groupFeedSubs(subs), [subs]);
	const showFolders = subGroups.some((group) => group.folder !== "");
	const totalUnread = subs.some((row) => row.unreadCount !== null)
		? subs.reduce((n, row) => n + (row.unreadCount ?? 0), 0)
		: null;
	const hasUnread =
		(selected ? selected.unreadCount : totalUnread) !== 0 ||
		items.some((row) => !row.readAt);
	const hasSubs = subs.length > 0;

	const detailPane = openItem ? (
		<PlazaFeedItemDetail
			item={openItem}
			hideSource={selectedId !== null}
			onBack={() => {
				setOpenItem(null);
				if (hideRead) void loadItems(selectedId);
			}}
			onToggleRead={() => void setRead(openItem, !openItem.readAt)}
			onImport={(item) => void importItem(item)}
			importing={importingIds.has(openItem.id)}
			onResolved={applyItem}
			resolveBody={cloud ? feedsCloudItem : undefined}
			starred={bookmarks.has(openItem.id)}
			onStar={() => onStar(openItem)}
		/>
	) : null;

	return (
		<div className={cn("flex h-full min-h-0 flex-col", className)}>
			{!hasSubs && (loading || source === null) ? (
				<div className="flex-1" />
			) : !hasSubs && cloud ? (
				<div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
					<Cloud className="size-5 text-muted-foreground/60" aria-hidden />
					<p className="text-muted-foreground text-sm">
						{t("plaza.feeds.emptyItems")}
					</p>
					{cloudButton}
				</div>
			) : !hasSubs ? (
				<div className="flex flex-1 flex-col items-center justify-center p-6">
					<div className="w-full max-w-md space-y-4 animate-in fade-in-0 duration-200">
						<div className="mx-auto flex size-12 items-center justify-center rounded-2xl bg-muted/60 text-muted-foreground">
							<Rss className="size-5" aria-hidden />
						</div>
						<p className="text-center font-medium text-sm">
							{t("plaza.feeds.emptyTitle")}
						</p>
						<AddForm
							busy={busy}
							refreshing={false}
							onAdd={(url) => void onAdd(url)}
							onRefresh={() => undefined}
							showRefresh={false}
							showChips
						/>
						<p className="text-center text-muted-foreground text-xs leading-relaxed">
							{t("plaza.feeds.emptyHint")}
						</p>
						<div className="flex justify-center">{cloudButton}</div>
					</div>
				</div>
			) : (
				<div ref={setPanesEl} className="flex min-h-0 min-w-0 flex-1">
					<nav className="flex w-52 shrink-0 select-none flex-col gap-2 bg-muted/50 p-2">
						{cloud ? null : (
							<AddForm
								busy={busy}
								refreshing={refreshing}
								onAdd={(url) => void onAdd(url)}
								onRefresh={() => void onRefresh()}
								showRefresh={false}
								showChips={false}
							/>
						)}
						<div className="flex shrink-0 items-center gap-1.5">
							<Button
								type="button"
								variant="ghost"
								size="sm"
								className="min-w-0 flex-1 bg-foreground/5 hover:bg-foreground/10"
								disabled={refreshing}
								onClick={() => void onRefresh()}
							>
								<RefreshCw
									data-icon="inline-start"
									className={cn(refreshing && "animate-spin")}
									aria-hidden
								/>
								<span className="min-w-0 truncate">
									{t("plaza.feeds.refresh")}
								</span>
							</Button>
							<Button
								type="button"
								variant="ghost"
								size="sm"
								aria-pressed={hideRead}
								className={cn(
									"min-w-0 flex-1 bg-foreground/5 hover:bg-foreground/10",
									hideRead && "bg-foreground/10 text-foreground",
								)}
								onClick={() => {
									hideReadRef.current = !hideRead;
									setHideRead(!hideRead);
									writeHideReadPreference(!hideRead);
									void loadItems(selectedId, false);
								}}
							>
								<EyeOff data-icon="inline-start" aria-hidden />
								<span className="min-w-0 truncate">
									{t("plaza.feeds.hideReadShort")}
								</span>
							</Button>
							<Tooltip>
								<TooltipTrigger asChild>
									<Button
										type="button"
										variant="ghost"
										size="icon-sm"
										aria-label={t("plaza.feeds.markAllRead")}
										className="shrink-0 bg-foreground/5 hover:bg-foreground/10"
										disabled={markingAll || !hasUnread}
										onClick={() => void onMarkAllRead()}
									>
										<CheckCheck className="size-3.5" aria-hidden />
									</Button>
								</TooltipTrigger>
								<TooltipContent>{t("plaza.feeds.markAllRead")}</TooltipContent>
							</Tooltip>
							<Tooltip>
								<TooltipTrigger asChild>
									<Button
										type="button"
										variant="ghost"
										size="icon-sm"
										aria-label={t("plaza.feeds.starredOnly")}
										aria-pressed={starredOnly}
										className={cn(
											"shrink-0 bg-foreground/5 hover:bg-foreground/10",
											starredOnly && "bg-foreground/10 text-amber-500",
										)}
										onClick={() => {
											starredOnlyRef.current = !starredOnly;
											setStarredOnly(!starredOnly);
											void loadItems(selectedId);
										}}
									>
										<Star
											className="size-3.5"
											fill={starredOnly ? "currentColor" : "none"}
											aria-hidden
										/>
									</Button>
								</TooltipTrigger>
								<TooltipContent>{t("plaza.feeds.starredOnly")}</TooltipContent>
							</Tooltip>
							{cloudButton}
						</div>
						<div className="agentero-scroll min-h-0 flex-1 space-y-0.5 overflow-y-auto">
							<button
								type="button"
								onClick={() => {
									setSelectedId(null);
									void loadItems(null);
								}}
								className={cn(
									"flex w-full items-center gap-1 rounded-lg px-2.5 py-1.5 text-left text-sm",
									"transition-colors duration-150 ease-out",
									"focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
									selectedId === null
										? "bg-foreground/10 text-foreground"
										: "text-muted-foreground hover:bg-foreground/5 hover:text-foreground",
								)}
							>
								<span className="min-w-0 flex-1 truncate">
									{t("plaza.feeds.all")}
								</span>
								<UnreadCount value={totalUnread} />
							</button>
							{subGroups.map((group) => (
								<Fragment key={`folder:${group.folder}`}>
									{showFolders ? (
										<button
											type="button"
											aria-expanded={!collapsedFolders.has(group.folder)}
											onClick={() =>
												setCollapsedFolders((prev) => {
													const next = new Set(prev);
													if (!next.delete(group.folder))
														next.add(group.folder);
													return next;
												})
											}
											className="flex w-full items-center gap-1 rounded-lg px-1.5 pt-2 pb-0.5 text-left text-muted-foreground text-xs hover:text-foreground"
										>
											<ChevronRight
												className={cn(
													"size-3 shrink-0 transition-transform duration-150",
													!collapsedFolders.has(group.folder) && "rotate-90",
												)}
												aria-hidden
											/>
											<span className="min-w-0 flex-1 truncate">
												{group.folder || t("plaza.feeds.noFolder")}
											</span>
											<UnreadCount
												value={
													group.subs.some((row) => row.unreadCount !== null)
														? group.subs.reduce(
																(n, row) => n + (row.unreadCount ?? 0),
																0,
															)
														: null
												}
											/>
										</button>
									) : null}
									{showFolders && collapsedFolders.has(group.folder)
										? null
										: group.subs.map((sub) => (
												<div key={sub.id} className="group/feed relative">
													<button
														type="button"
														onClick={() => {
															setSelectedId(sub.id);
															void loadItems(sub.id);
														}}
														onContextMenu={(event) =>
															openFeedSubMenu(sub, event, setSubMenu)
														}
														className={cn(
															"flex w-full flex-col rounded-lg py-1.5 pr-7 pl-2.5 text-left",
															"transition-colors duration-150 ease-out",
															"focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
															selectedId === sub.id
																? "bg-foreground/10 text-foreground"
																: "hover:bg-foreground/5",
														)}
													>
														<span className="flex min-w-0 items-center gap-1">
															{sub.pinned ? (
																<Pin
																	className="size-3 shrink-0 text-muted-foreground"
																	aria-hidden
																/>
															) : null}
															<span className="min-w-0 flex-1 truncate text-sm">
																{sub.title}
															</span>
															<UnreadCount value={sub.unreadCount} />
														</span>
														{sub.lastError ? (
															<span className="truncate text-destructive text-xs">
																{formatHostError(sub.lastError, t)}
															</span>
														) : null}
													</button>
													{cloud ? null : (
														<Button
															type="button"
															variant="ghost"
															size="icon-sm"
															className="absolute top-0 right-0.5 bottom-0 my-auto size-6 opacity-0 transition-opacity duration-150 group-hover/feed:opacity-100 focus-visible:opacity-100"
															aria-label={t("plaza.feeds.more")}
															onClick={(event) => {
																event.stopPropagation();
																const rect =
																	event.currentTarget.getBoundingClientRect();
																setSubMenu({
																	sub,
																	x: rect.right,
																	y: rect.bottom,
																});
															}}
														>
															<EllipsisVertical
																className="size-3.5"
																aria-hidden
															/>
														</Button>
													)}
												</div>
											))}
								</Fragment>
							))}
						</div>
					</nav>
					{openItem && !widePanes ? (
						<div className="flex min-h-0 min-w-0 flex-1 flex-col">
							{detailPane}
						</div>
					) : (
						<>
							<div
								className={cn(
									"flex min-h-0 min-w-0 flex-col",
									openItem ? "w-[22rem] shrink-0 border-r" : "flex-1",
								)}
							>
								{emptyItems ? (
									<div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center">
										<Inbox
											className="size-5 text-muted-foreground/60"
											aria-hidden
										/>
										<p className="text-muted-foreground text-sm">
											{selected?.lastError
												? formatHostError(selected.lastError, t)
												: t(
														starredOnly
															? "plaza.feeds.emptyStarred"
															: hideRead
																? "plaza.feeds.emptyUnread"
																: "plaza.feeds.emptyItems",
													)}
										</p>
									</div>
								) : (
									<div
										ref={listRef}
										className="agentero-scroll min-h-0 flex-1 overflow-y-auto px-3 pt-2 pb-3"
									>
										<div
											className="relative w-full"
											style={{ height: virtualizer.getTotalSize() }}
										>
											{virtualRows.map((virtualRow) => {
												const item = items[virtualRow.index];
												return (
													<div
														key={item.id}
														className="absolute top-0 left-0 w-full pb-2"
														style={{
															transform: `translateY(${virtualRow.start}px)`,
														}}
														ref={virtualizer.measureElement}
														data-index={virtualRow.index}
													>
														<PlazaFeedItemRow
															item={item}
															starred={bookmarks.has(item.id)}
															onStar={onStar}
															hideSource={selectedId !== null}
															onOpen={openFeedItem}
															onImport={(item) => void importItem(item)}
															importing={importingIds.has(item.id)}
															onToggleRead={(item) =>
																void setRead(item, !item.readAt)
															}
														/>
													</div>
												);
											})}
										</div>
									</div>
								)}
							</div>
							{openItem ? (
								<div className="flex min-h-0 min-w-0 flex-1 flex-col">
									{detailPane}
								</div>
							) : null}
						</>
					)}
				</div>
			)}

			{subMenu ? (
				<FeedSubMenu
					menu={subMenu}
					cloud={cloud}
					onClose={closeSubMenu}
					onMarkRead={(sub) => void onMarkAllRead(sub.id)}
					onRename={(sub) => {
						setRenameTarget(sub);
						setRenameTitle(sub.title);
					}}
					onCopyUrl={(sub) =>
						void copyTextToClipboard(sub.url, {
							successMessage: t("plaza.feeds.copiedUrl"),
						})
					}
					onRefresh={(sub) =>
						void feedsRefresh({ id: sub.id }).then((result) => {
							setSubs([...result.subscriptions].sort(compareFeedSubs));
							void loadItems(selectedId);
						})
					}
					onPin={(sub) => void onPin(sub.id, !sub.pinned)}
					onRemove={(sub) => void onRemove(sub.id)}
				/>
			) : null}

			<PlazaFeedBookmarkDialog
				item={bookmarkTarget}
				bookmark={
					bookmarkTarget ? (bookmarks.get(bookmarkTarget.id) ?? null) : null
				}
				vaultPath={vaultPath}
				onClose={() => setBookmarkTarget(null)}
				onConfirm={confirmBookmark}
				onOpenNote={(bookmark) => {
					setBookmarkTarget(null);
					openVaultRel(bookmark.notePath);
				}}
				onRemove={removeBookmark}
			/>

			<CloudSourceDialog
				open={cloudDialogOpen}
				settings={cloudSettings}
				onOpenChange={setCloudDialogOpen}
				onSaved={setCloudSettings}
			/>

			<Dialog
				open={renameTarget !== null}
				onOpenChange={(open) => {
					if (!open) setRenameTarget(null);
				}}
			>
				<DialogContent className="max-w-sm">
					<DialogHeader>
						<DialogTitle>{t("plaza.feeds.rename")}</DialogTitle>
					</DialogHeader>
					<Input
						value={renameTitle}
						onChange={(event) => setRenameTitle(event.target.value)}
						onKeyDown={(event) => {
							if (event.key === "Enter") {
								event.preventDefault();
								void onRename();
							}
						}}
						aria-label={t("plaza.feeds.rename")}
					/>
					<DialogFooter>
						<Button
							type="button"
							variant="ghost"
							onClick={() => setRenameTarget(null)}
						>
							{t("plaza.feeds.cancel")}
						</Button>
						<Button type="button" onClick={() => void onRename()}>
							{t("plaza.feeds.save")}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</div>
	);
}
