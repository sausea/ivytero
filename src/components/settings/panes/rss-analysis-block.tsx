import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
	SettingsGroup,
	SettingsRow,
	SettingsSectionLabel,
} from "@/components/settings/settings-layout";
import { Input } from "@/components/ui/input";
import type { AppSettings, RssAnalysisSettings } from "@/lib/settings";

/** OpenAI-compatible endpoint for RSS bookmark AI analysis (BYOK). */
export function RssAnalysisBlock({
	settings,
	patch,
}: {
	settings: AppSettings;
	patch: (p: Partial<AppSettings>) => void;
}) {
	const { t } = useTranslation("settings");
	const cfg = settings.rssAnalysis;
	const [draft, setDraft] = useState<RssAnalysisSettings>(() => ({ ...cfg }));
	useEffect(() => {
		setDraft({ ...cfg });
	}, [cfg]);
	const commit = (next: Partial<RssAnalysisSettings>) => {
		patch({ rssAnalysis: { ...settings.rssAnalysis, ...next } });
	};
	const inputClass =
		"h-8 w-56 font-mono text-xs placeholder:text-muted-foreground/50";

	return (
		<>
			<div className="mb-1.5 mt-4">
				<SettingsSectionLabel className="mb-0 px-0">
					{t("rssAnalysis.section")}
				</SettingsSectionLabel>
			</div>
			<SettingsGroup>
				<SettingsRow
					label={t("rssAnalysis.baseUrl")}
					description={t("rssAnalysis.baseUrlHint")}
					htmlFor="rss-analysis-base-url"
				>
					<Input
						id="rss-analysis-base-url"
						value={draft.baseUrl}
						onChange={(e) => setDraft({ ...draft, baseUrl: e.target.value })}
						onBlur={() => {
							const trimmed = draft.baseUrl.trim().replace(/\/+$/, "");
							if (trimmed !== settings.rssAnalysis.baseUrl) {
								commit({ baseUrl: trimmed });
							}
						}}
						onKeyDown={(e) => {
							if (e.key === "Enter") e.currentTarget.blur();
						}}
						placeholder="https://api.openai.com/v1"
						spellCheck={false}
						autoComplete="off"
						className={inputClass}
					/>
				</SettingsRow>
				<SettingsRow
					label={t("rssAnalysis.apiKey")}
					htmlFor="rss-analysis-api-key"
				>
					<Input
						id="rss-analysis-api-key"
						type="password"
						value={draft.apiKey}
						onChange={(e) => setDraft({ ...draft, apiKey: e.target.value })}
						onBlur={() => {
							const next = draft.apiKey.trim();
							if (next !== settings.rssAnalysis.apiKey) {
								commit({ apiKey: next });
							}
						}}
						onKeyDown={(e) => {
							if (e.key === "Enter") e.currentTarget.blur();
						}}
						placeholder="sk-…"
						spellCheck={false}
						autoComplete="off"
						className={inputClass}
					/>
				</SettingsRow>
				<SettingsRow
					label={t("rssAnalysis.model")}
					htmlFor="rss-analysis-model"
				>
					<Input
						id="rss-analysis-model"
						value={draft.model}
						onChange={(e) => setDraft({ ...draft, model: e.target.value })}
						onBlur={() => {
							const trimmed = draft.model.trim();
							if (trimmed !== settings.rssAnalysis.model) {
								commit({ model: trimmed });
							}
						}}
						onKeyDown={(e) => {
							if (e.key === "Enter") e.currentTarget.blur();
						}}
						placeholder="gpt-4o-mini"
						spellCheck={false}
						autoComplete="off"
						className={inputClass}
					/>
				</SettingsRow>
			</SettingsGroup>
		</>
	);
}
