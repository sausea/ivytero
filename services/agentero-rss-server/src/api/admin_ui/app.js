(() => {
	const zh = (navigator.language || "").toLowerCase().startsWith("zh");
	const T = zh
		? {
				title: "Agentero 云端 RSS",
				loginHint:
					"输入服务端的 RSS_ADMIN_TOKEN 登录。Token 只保存在本标签页。",
				adminToken: "管理员 Token",
				signIn: "登录",
				signOut: "退出",
				feeds: "订阅",
				articles: "文章",
				feedUrl: "Feed 地址",
				titleOptional: "标题（可选）",
				add: "添加",
				importOpml: "导入 OPML",
				exportOpml: "导出 OPML",
				reload: "刷新列表",
				feed: "订阅",
				status: "状态",
				articleCount: "文章数",
				interval: "间隔",
				lastSuccess: "最近成功",
				enabled: "启用",
				disabled: "已停用",
				failing: "失败 {n} 次",
				refresh: "立即抓取",
				disable: "停用",
				enable: "启用",
				editInterval: "改间隔",
				rename: "重命名",
				never: "从未",
				summary: "{feeds} 个订阅 · {articles} 篇文章",
				allFeeds: "全部订阅",
				loadMore: "加载更多",
				noArticles: "没有文章。",
				pick: "选择一篇文章查看。",
				openOriginal: "打开原文",
				added: "已添加",
				queued: "已加入抓取队列",
				imported:
					"导入完成：新增 {added}，跳过 {skipped}（{dup} 个重复，{bad} 个无效）",
				intervalPrompt: "抓取间隔（秒，120–86400）",
				titlePrompt: "新标题",
				unauthorized: "Token 无效。",
				failed: "操作失败：",
				disableConfirm: "停用后不再抓取，历史文章保留。继续？",
			}
		: {
				title: "Agentero Cloud RSS",
				loginHint:
					"Sign in with the server's RSS_ADMIN_TOKEN. It is kept in this tab only.",
				adminToken: "Admin token",
				signIn: "Sign in",
				signOut: "Sign out",
				feeds: "Feeds",
				articles: "Articles",
				feedUrl: "Feed URL",
				titleOptional: "Title (optional)",
				add: "Add",
				importOpml: "Import OPML",
				exportOpml: "Export OPML",
				reload: "Reload",
				feed: "Feed",
				status: "Status",
				articleCount: "Articles",
				interval: "Interval",
				lastSuccess: "Last success",
				enabled: "Enabled",
				disabled: "Disabled",
				failing: "{n} failures",
				refresh: "Fetch now",
				disable: "Disable",
				enable: "Enable",
				editInterval: "Interval",
				rename: "Rename",
				never: "never",
				summary: "{feeds} feeds · {articles} articles",
				allFeeds: "All feeds",
				loadMore: "Load more",
				noArticles: "No articles.",
				pick: "Pick an article to read it.",
				openOriginal: "Open original",
				added: "Added",
				queued: "Queued for fetching",
				imported:
					"Imported: {added} added, {skipped} skipped ({dup} duplicate, {bad} invalid)",
				intervalPrompt: "Poll interval (seconds, 120–86400)",
				titlePrompt: "New title",
				unauthorized: "Invalid token.",
				failed: "Failed: ",
				disableConfirm: "Stop polling this feed? Its history is kept.",
			};
	const t = (k, vars = {}) =>
		(T[k] || k).replace(/\{(\w+)\}/g, (_, v) => String(vars[v] ?? ""));
	const $ = (id) => document.getElementById(id);
	const el = (tag, props = {}, ...kids) => {
		const n = document.createElement(tag);
		for (const [k, v] of Object.entries(props)) {
			if (k === "class") n.className = v;
			else if (k === "on")
				for (const [e, f] of Object.entries(v)) n.addEventListener(e, f);
			else n[k] = v;
		}
		for (const kid of kids) if (kid != null) n.append(kid);
		return n;
	};
	document.documentElement.lang = zh ? "zh-CN" : "en";
	for (const n of document.querySelectorAll("[data-i18n]"))
		n.textContent = t(n.dataset.i18n);
	for (const n of document.querySelectorAll("[data-i18n-placeholder]"))
		n.placeholder = t(n.dataset.i18nPlaceholder);
	document.title = t("title");

	let token = sessionStorage.getItem("rssAdminToken") || "";
	let feeds = [];
	let cursor = null;
	let toastTimer = 0;

	function toast(msg, bad = false) {
		const n = $("toast");
		n.textContent = msg;
		n.className = bad ? "bad" : "";
		n.hidden = false;
		clearTimeout(toastTimer);
		toastTimer = setTimeout(() => (n.hidden = true), bad ? 6000 : 3000);
	}

	async function api(path, opts = {}) {
		const res = await fetch(`/api/admin${path}`, {
			...opts,
			headers: { Authorization: `Bearer ${token}`, ...(opts.headers || {}) },
		});
		if (res.status === 401) {
			signOut();
			throw new Error(t("unauthorized"));
		}
		if (!res.ok) {
			let msg = `${res.status}`;
			try {
				msg = (await res.json()).error.message;
			} catch {
				/* non-JSON error */
			}
			throw new Error(msg);
		}
		if (res.status === 204) return null;
		const type = res.headers.get("content-type") || "";
		return type.includes("json") ? res.json() : res.text();
	}
	const fail = (e) =>
		toast(
			e.message.startsWith(t("unauthorized"))
				? e.message
				: t("failed") + e.message,
			true,
		);

	function signOut() {
		token = "";
		sessionStorage.removeItem("rssAdminToken");
		$("app").hidden = true;
		$("login").hidden = false;
		$("token").focus();
	}
	async function signIn() {
		token = $("token").value.trim();
		if (!token) return;
		try {
			const info = await api("/info");
			sessionStorage.setItem("rssAdminToken", token);
			$("token").value = "";
			$("serverId").textContent = info.serverId;
			$("login").hidden = true;
			$("app").hidden = false;
			await loadFeeds();
		} catch (e) {
			fail(e);
		}
	}

	const when = (iso) =>
		iso
			? new Date(iso).toLocaleString(zh ? "zh-CN" : undefined, {
					dateStyle: "short",
					timeStyle: "short",
				})
			: t("never");
	const human = (s) =>
		s % 3600 === 0 ? `${s / 3600}h` : s % 60 === 0 ? `${s / 60}m` : `${s}s`;

	async function loadFeeds() {
		try {
			feeds = (await api("/feeds")).items;
		} catch (e) {
			return fail(e);
		}
		const total = feeds.reduce((n, f) => n + f.articleCount, 0);
		$("feedSummary").textContent = t("summary", {
			feeds: feeds.length,
			articles: total,
		});
		const rows = $("feedRows");
		rows.replaceChildren(
			...feeds.map((f) => {
				const status = !f.enabled
					? el("span", { class: "pill off", textContent: t("disabled") })
					: f.failureCount > 0
						? el("span", {
								class: "pill bad",
								textContent: t("failing", { n: f.failureCount }),
							})
						: el("span", { class: "pill on", textContent: t("enabled") });
				const act = (label, fn) =>
					el("button", {
						textContent: t(label),
						on: { click: () => fn().catch(fail) },
					});
				return el(
					"tr",
					{},
					el(
						"td",
						{},
						el("strong", { textContent: f.title }),
						el("span", { class: "url", textContent: f.url }),
						f.lastError
							? el("span", { class: "err", textContent: f.lastError })
							: null,
					),
					el("td", {}, status),
					el("td", { textContent: String(f.articleCount) }),
					el("td", { textContent: human(f.refreshIntervalSecs) }),
					el("td", { textContent: when(f.lastSuccessAt) }),
					el(
						"td",
						{ class: "actions" },
						act("refresh", async () => {
							await api(`/feeds/${f.id}/refresh`, { method: "POST" });
							toast(t("queued"));
						}),
						act("rename", async () => {
							const title = prompt(t("titlePrompt"), f.title);
							if (title?.trim()) {
								await patch(f.id, { title });
							}
						}),
						act("editInterval", async () => {
							const raw = prompt(
								t("intervalPrompt"),
								String(f.refreshIntervalSecs),
							);
							if (raw?.trim()) await patch(f.id, { intervalSecs: Number(raw) });
						}),
						act(f.enabled ? "disable" : "enable", async () => {
							if (f.enabled && !confirm(t("disableConfirm"))) return;
							await patch(f.id, { enabled: !f.enabled });
						}),
					),
				);
			}),
		);
		const sel = $("articleFeed");
		const current = sel.value;
		sel.replaceChildren(
			el("option", { value: "", textContent: t("allFeeds") }),
			...feeds.map((f) => el("option", { value: f.id, textContent: f.title })),
		);
		sel.value = feeds.some((f) => f.id === current) ? current : "";
	}

	async function patch(id, body) {
		await api(`/feeds/${id}`, {
			method: "PATCH",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(body),
		});
		await loadFeeds();
	}

	async function addFeed() {
		const url = $("newUrl").value.trim();
		if (!url) return;
		const title = $("newTitle").value.trim();
		try {
			await api("/feeds", {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({ url, title: title || null }),
			});
			$("newUrl").value = "";
			$("newTitle").value = "";
			toast(t("added"));
			await loadFeeds();
		} catch (e) {
			fail(e);
		}
	}

	async function importOpml(file) {
		if (!file) return;
		try {
			const text = await file.text();
			const r = await api("/opml", {
				method: "POST",
				headers: { "Content-Type": "text/x-opml" },
				body: text,
			});
			const dup = r.skipped.filter((s) => s.reason === "duplicate").length;
			toast(
				t("imported", {
					added: r.added.length,
					skipped: r.skipped.length,
					dup,
					bad: r.skipped.length - dup,
				}),
			);
			await loadFeeds();
		} catch (e) {
			fail(e);
		} finally {
			$("opmlFile").value = "";
		}
	}

	async function exportOpml() {
		try {
			const xml = await api("/opml");
			const a = el("a", {
				href: URL.createObjectURL(new Blob([xml], { type: "text/x-opml" })),
				download: "agentero-rss.opml",
			});
			document.body.append(a);
			a.click();
			a.remove();
			setTimeout(() => URL.revokeObjectURL(a.href), 1000);
		} catch (e) {
			fail(e);
		}
	}

	// Archived HTML is untrusted: rebuild it from an allow-list, never inject it.
	const ALLOWED = new Set([
		"P",
		"BR",
		"A",
		"UL",
		"OL",
		"LI",
		"H1",
		"H2",
		"H3",
		"H4",
		"H5",
		"H6",
		"PRE",
		"CODE",
		"BLOCKQUOTE",
		"EM",
		"STRONG",
		"B",
		"I",
		"IMG",
		"TABLE",
		"THEAD",
		"TBODY",
		"TR",
		"TD",
		"TH",
		"FIGURE",
		"FIGCAPTION",
		"HR",
		"SPAN",
		"DIV",
		"SUP",
		"SUB",
	]);
	const safeUrl = (raw) => {
		try {
			const u = new URL(raw, "https://invalid.example/");
			return /^https?:$/.test(u.protocol) && u.host !== "invalid.example"
				? u.href
				: null;
		} catch {
			return null;
		}
	};
	function sanitize(node) {
		const frag = document.createDocumentFragment();
		for (const child of node.childNodes) {
			if (child.nodeType === Node.TEXT_NODE) {
				frag.append(child.textContent);
				continue;
			}
			if (child.nodeType !== Node.ELEMENT_NODE) continue;
			if (!ALLOWED.has(child.tagName)) {
				if (
					![
						"SCRIPT",
						"STYLE",
						"IFRAME",
						"OBJECT",
						"EMBED",
						"FORM",
						"SVG",
						"MATH",
					].includes(child.tagName)
				)
					frag.append(sanitize(child));
				continue;
			}
			const out = document.createElement(child.tagName);
			if (child.tagName === "A") {
				const href = safeUrl(child.getAttribute("href") || "");
				if (href) {
					out.href = href;
					out.target = "_blank";
					out.rel = "noopener noreferrer";
				}
			}
			if (child.tagName === "IMG") {
				const src = safeUrl(child.getAttribute("src") || "");
				if (!src) continue;
				out.src = src;
				out.alt = child.getAttribute("alt") || "";
				out.loading = "lazy";
				out.referrerPolicy = "no-referrer";
			}
			out.append(sanitize(child));
			frag.append(out);
		}
		return frag;
	}

	async function loadArticles(reset) {
		if (reset) {
			cursor = null;
			$("articleList").replaceChildren();
			$("articleView").replaceChildren(
				el("p", { class: "muted", textContent: t("pick") }),
			);
		}
		const feedId = $("articleFeed").value;
		const q = new URLSearchParams({ limit: "50" });
		if (feedId) q.set("feedId", feedId);
		if (cursor) q.set("cursor", cursor);
		try {
			const page = await api(`/articles?${q}`);
			cursor = page.nextCursor;
			$("moreBtn").hidden = !cursor;
			const list = $("articleList");
			if (reset && page.items.length === 0)
				list.append(el("li", { class: "muted", textContent: t("noArticles") }));
			for (const a of page.items) {
				const li = el(
					"li",
					{ on: { click: () => openArticle(a.id, li) } },
					el("div", { textContent: a.title }),
					el("div", {
						class: "meta",
						textContent: `${a.feedTitle} · ${when(a.publishedAt || a.firstSeenAt)}`,
					}),
				);
				list.append(li);
			}
		} catch (e) {
			fail(e);
		}
	}

	async function openArticle(id, li) {
		for (const n of $("articleList").children)
			n.classList.toggle("active", n === li);
		try {
			const a = await api(`/articles/${encodeURIComponent(id)}`);
			const view = $("articleView");
			const href = a.url && safeUrl(a.url);
			const body = el("div");
			if (a.contentHtml)
				body.append(
					sanitize(
						new DOMParser().parseFromString(a.contentHtml, "text/html").body,
					),
				);
			else body.append(el("p", { textContent: a.summaryText }));
			view.replaceChildren(
				el("h1", { textContent: a.title }),
				el(
					"p",
					{ class: "muted" },
					`${a.feedTitle} · ${when(a.publishedAt || a.firstSeenAt)} `,
					href
						? el("a", {
								href,
								target: "_blank",
								rel: "noopener noreferrer",
								textContent: t("openOriginal"),
							})
						: null,
				),
				body,
			);
			view.scrollTop = 0;
		} catch (e) {
			fail(e);
		}
	}

	function showTab(name) {
		for (const b of document.querySelectorAll(".tab"))
			b.classList.toggle("active", b.dataset.tab === name);
		$("tab-feeds").hidden = name !== "feeds";
		$("tab-articles").hidden = name !== "articles";
		if (name === "articles" && !$("articleList").children.length)
			loadArticles(true);
	}

	$("loginBtn").addEventListener("click", signIn);
	$("token").addEventListener("keydown", (e) => {
		if (e.key === "Enter") signIn();
	});
	$("logout").addEventListener("click", signOut);
	$("addBtn").addEventListener("click", addFeed);
	$("newUrl").addEventListener("keydown", (e) => {
		if (e.key === "Enter") addFeed();
	});
	$("opmlFile").addEventListener("change", (e) =>
		importOpml(e.target.files[0]),
	);
	$("exportBtn").addEventListener("click", exportOpml);
	$("reloadFeeds").addEventListener("click", loadFeeds);
	$("articleFeed").addEventListener("change", () => loadArticles(true));
	$("moreBtn").addEventListener("click", () => loadArticles(false));
	for (const b of document.querySelectorAll(".tab"))
		b.addEventListener("click", () => showTab(b.dataset.tab));

	if (token) {
		$("token").value = token;
		signIn();
	} else signOut();
})();
