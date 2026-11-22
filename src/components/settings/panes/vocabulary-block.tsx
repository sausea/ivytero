import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
	SettingsGroup,
	SettingsRow,
	SettingsSectionLabel,
} from "@/components/settings/settings-layout";
import { Input } from "@/components/ui/input";
import type { AppSettings, VocabularySettings } from "@/lib/settings";

/** OpenAI-compatible endpoint for the vocabulary notebook dictionary (BYOK). */
export function VocabularyBlock({
	settings,
	patch,
}: {
	settings: AppSettings;
	patch: (p: Partial<AppSettings>) => void;
}) {
	const { t } = useTranslation("settings");
	const cfg = settings.vocabulary;
	const [draft, setDraft] = useState<VocabularySettings>(() => ({ ...cfg }));
	useEffect(() => {
		setDraft({ ...cfg });
	}, [cfg]);
	const commit = (next: Partial<VocabularySettings>) => {
		patch({ vocabulary: { ...settings.vocabulary, ...next } });
	};
	const inputClass =
		"h-8 w-56 font-mono text-xs placeholder:text-muted-foreground/50";

	return (
		<>
			<div className="mb-1.5 mt-4">
				<SettingsSectionLabel className="mb-0 px-0">
					{t("vocabulary.section")}
				</SettingsSectionLabel>
			</div>
			<SettingsGroup>
				<SettingsRow
					label={t("vocabulary.baseUrl")}
					description={t("vocabulary.baseUrlHint")}
					htmlFor="vocabulary-base-url"
				>
					<Input
						id="vocabulary-base-url"
						value={draft.baseUrl}
						onChange={(e) => setDraft({ ...draft, baseUrl: e.target.value })}
						onBlur={() => {
							const trimmed = draft.baseUrl.trim().replace(/\/+$/, "");
							if (trimmed !== settings.vocabulary.baseUrl) {
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
					label={t("vocabulary.apiKey")}
					htmlFor="vocabulary-api-key"
				>
					<Input
						id="vocabulary-api-key"
						type="password"
						value={draft.apiKey}
						onChange={(e) => setDraft({ ...draft, apiKey: e.target.value })}
						onBlur={() => {
							const next = draft.apiKey.trim();
							if (next !== settings.vocabulary.apiKey) {
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
				<SettingsRow label={t("vocabulary.model")} htmlFor="vocabulary-model">
					<Input
						id="vocabulary-model"
						value={draft.model}
						onChange={(e) => setDraft({ ...draft, model: e.target.value })}
						onBlur={() => {
							const trimmed = draft.model.trim();
							if (trimmed !== settings.vocabulary.model) {
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
