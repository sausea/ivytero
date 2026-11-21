/**
 * Star → "bookmark to vault" panel: human tags (at least one), target vault
 * and daily note preview. For an existing bookmark it offers open / remove.
 */
import { FileText, Loader2, Star, Trash2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import type { FeedItem } from "@/lib/plaza/feeds";
import {
	localArchiveStamp,
	MAX_BOOKMARK_TAG_CHARS,
	MAX_BOOKMARK_TAGS,
	normalizeBookmarkTag,
	type RssBookmark,
	rssDailyNotePath,
} from "@/lib/plaza/rss-bookmarks";
import { vaultDisplayName } from "@/lib/vault/path";

export function PlazaFeedBookmarkDialog({
	item,
	bookmark,
	vaultPath,
	onClose,
	onConfirm,
	onOpenNote,
	onRemove,
}: {
	item: FeedItem | null;
	bookmark: RssBookmark | null;
	vaultPath: string | null;
	onClose: () => void;
	onConfirm: (item: FeedItem, tags: string[]) => Promise<void>;
	onOpenNote: (bookmark: RssBookmark) => void;
	onRemove: (bookmark: RssBookmark) => Promise<void>;
}) {
	const { t } = useTranslation("sidebar");
	const [tags, setTags] = useState<string[]>([]);
	const [draft, setDraft] = useState("");
	const [busy, setBusy] = useState(false);
	const [confirmRemove, setConfirmRemove] = useState(false);
	const open = item !== null;

	useEffect(() => {
		if (!open) return;
		setTags([]);
		setDraft("");
		setBusy(false);
		setConfirmRemove(false);
	}, [open]);

	const addDraft = (): string[] => {
		const tag = normalizeBookmarkTag(draft).slice(0, MAX_BOOKMARK_TAG_CHARS);
		setDraft("");
		if (!tag || tags.includes(tag) || tags.length >= MAX_BOOKMARK_TAGS) {
			return tags;
		}
		const next = [...tags, tag];
		setTags(next);
		return next;
	};

	const notePath =
		bookmark?.notePath ?? rssDailyNotePath(localArchiveStamp().date);
	const confirm = async () => {
		if (!item) return;
		const next = draft.trim() ? addDraft() : tags;
		if (next.length === 0) return;
		setBusy(true);
		try {
			await onConfirm(item, next);
		} finally {
			setBusy(false);
		}
	};
	const remove = async () => {
		if (!bookmark) return;
		if (!confirmRemove) {
			setConfirmRemove(true);
			return;
		}
		setBusy(true);
		try {
			await onRemove(bookmark);
		} finally {
			setBusy(false);
		}
	};
	const shownTags = bookmark ? bookmark.tags : tags;

	return (
		<Dialog
			open={open}
			onOpenChange={(next) => {
				if (!next) onClose();
			}}
		>
			<DialogContent className="max-w-md">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-1.5">
						<Star
							className="size-4"
							fill={bookmark ? "currentColor" : "none"}
							aria-hidden
						/>
						{t(bookmark ? "plaza.feeds.starred" : "plaza.feeds.star")}
					</DialogTitle>
					<DialogDescription className="line-clamp-2">
						{[item?.title, item?.subscriptionTitle].filter(Boolean).join(" · ")}
					</DialogDescription>
				</DialogHeader>
				<div className="space-y-2">
					{shownTags.length > 0 ? (
						<div className="flex flex-wrap gap-1">
							{shownTags.map((tag) => (
								<Badge key={tag} variant="secondary" className="gap-0.5">
									{tag}
									{bookmark ? null : (
										<button
											type="button"
											className="rounded-sm opacity-60 hover:opacity-100"
											aria-label={`${t("plaza.feeds.bookmarkRemove")} ${tag}`}
											onClick={() => setTags(tags.filter((row) => row !== tag))}
										>
											<X className="size-3" aria-hidden />
										</button>
									)}
								</Badge>
							))}
						</div>
					) : null}
					{bookmark ? null : (
						<Input
							autoFocus
							value={draft}
							maxLength={MAX_BOOKMARK_TAG_CHARS}
							placeholder={t("plaza.feeds.bookmarkTagsPlaceholder")}
							aria-label={t("plaza.feeds.bookmarkTags")}
							onChange={(event) => setDraft(event.target.value)}
							onKeyDown={(event) => {
								if (event.nativeEvent.isComposing) return;
								if (event.key === "Enter" || event.key === ",") {
									event.preventDefault();
									if (
										event.key === "Enter" &&
										!draft.trim() &&
										tags.length > 0
									) {
										void confirm();
									} else {
										addDraft();
									}
								} else if (event.key === "Backspace" && !draft) {
									setTags(tags.slice(0, -1));
								}
							}}
						/>
					)}
					<p className="flex min-w-0 items-center gap-1 text-muted-foreground text-xs">
						<FileText className="size-3 shrink-0" aria-hidden />
						<span className="truncate">
							{t("plaza.feeds.bookmarkTarget", {
								vault: vaultDisplayName(vaultPath),
								path: notePath,
							})}
						</span>
					</p>
				</div>
				<DialogFooter>
					{bookmark ? (
						<>
							<Button
								type="button"
								variant="ghost"
								className="text-destructive hover:text-destructive"
								disabled={busy}
								onClick={() => void remove()}
							>
								<Trash2 data-icon="inline-start" aria-hidden />
								{t(
									confirmRemove
										? "plaza.feeds.bookmarkRemoveConfirm"
										: "plaza.feeds.bookmarkRemove",
								)}
							</Button>
							<Button type="button" onClick={() => onOpenNote(bookmark)}>
								{t("plaza.feeds.bookmarkOpen")}
							</Button>
						</>
					) : (
						<>
							<Button type="button" variant="ghost" onClick={onClose}>
								{t("plaza.feeds.cancel")}
							</Button>
							<Button
								type="button"
								disabled={busy || (tags.length === 0 && !draft.trim())}
								onClick={() => void confirm()}
							>
								{busy ? (
									<Loader2
										data-icon="inline-start"
										className="animate-spin"
										aria-hidden
									/>
								) : null}
								{t("plaza.feeds.bookmarkConfirm")}
							</Button>
						</>
					)}
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
