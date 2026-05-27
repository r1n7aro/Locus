import { access, readFile } from "node:fs/promises";
import path from "node:path";

export const releaseNoteChannelConfigs = {
  stable: {
    required: true,
    locales: {
      zh: {
        sourcePath: "overview/latest-version.mdx",
        changesHeading: "## 变更列表",
        downloadsHeadings: ["## 下载", "## 下载渠道"],
        expectedChangelogUrl: "/overview/latest-version",
      },
      en: {
        sourcePath: "en/overview/latest-version.mdx",
        changesHeading: "## Changes",
        downloadsHeadings: ["## Download", "## Download Channels"],
        expectedChangelogUrl: "/en/overview/latest-version",
      },
    },
  },
  experimental: {
    required: false,
    locales: {
      zh: {
        sourcePath: "overview/experimental-version.mdx",
        changesHeading: "## 变更列表",
        downloadsHeadings: ["## 下载", "## 下载渠道"],
        expectedChangelogUrl: "/overview/experimental-version",
      },
      en: {
        sourcePath: "en/overview/experimental-version.mdx",
        changesHeading: "## Changes",
        downloadsHeadings: ["## Download", "## Download Channels"],
        expectedChangelogUrl: "/en/overview/experimental-version",
      },
    },
  },
};

export const releaseNoteConfigs = releaseNoteChannelConfigs.stable.locales;
export const releaseNoteSharedFrontmatterKeys = ["version", "releasedAt", "channel"];
export const releaseNoteLocalizedFrontmatterKeys = [
  "title",
  "description",
  "sidebarTitle",
  "updateTitle",
  "changelogUrl",
];

const releaseAssetRepo = "r1n7aro/Locus";

const releaseNoteRequiredFrontmatterKeys = [
  ...releaseNoteSharedFrontmatterKeys,
  ...releaseNoteLocalizedFrontmatterKeys,
];

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

function normalizeReleaseNoteChannel(channel) {
  return channel === "experimental" ? "experimental" : "stable";
}

function releaseNoteChannelConfig(channel) {
  const normalizedChannel = normalizeReleaseNoteChannel(channel);
  return {
    channel: normalizedChannel,
    config: releaseNoteChannelConfigs[normalizedChannel],
  };
}

async function fileExists(filePath) {
  try {
    await access(filePath);
    return true;
  } catch {
    return false;
  }
}

function stripQuotes(value) {
  const trimmed = value.trim();
  if (
    (trimmed.startsWith('"') && trimmed.endsWith('"')) ||
    (trimmed.startsWith("'") && trimmed.endsWith("'"))
  ) {
    return trimmed.slice(1, -1);
  }
  return trimmed;
}

function parseFrontmatter(raw, filePath) {
  const match = raw.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?/);
  assert(match, `${filePath} 缺少 frontmatter`);

  const frontmatter = {};
  for (const line of match[1].split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed) {
      continue;
    }

    const separatorIndex = trimmed.indexOf(":");
    assert(separatorIndex > 0, `${filePath} 的 frontmatter 行格式无效：${trimmed}`);

    const key = trimmed.slice(0, separatorIndex).trim();
    const value = trimmed.slice(separatorIndex + 1).trim();
    frontmatter[key] = stripQuotes(value);
  }

  return {
    frontmatter,
    body: raw.slice(match[0].length),
  };
}

function parseChanges(body, filePath, changesHeading) {
  const lines = body.split(/\r?\n/);
  const headingIndex = lines.findIndex((line) => line.trim() === changesHeading);
  assert(headingIndex >= 0, `${filePath} 缺少 ${changesHeading}`);

  const changes = [];
  let currentGroup = null;

  for (let index = headingIndex + 1; index < lines.length; index += 1) {
    const trimmed = lines[index].trim();
    if (!trimmed) {
      continue;
    }

    if (trimmed.startsWith("## ")) {
      break;
    }

    if (trimmed.startsWith("### ")) {
      currentGroup = {
        title: trimmed.slice(4).trim(),
        items: [],
      };
      changes.push(currentGroup);
      continue;
    }

    if (trimmed.startsWith("- ")) {
      assert(currentGroup, `${filePath} 中的变更条目必须位于三级标题下`);
      currentGroup.items.push(trimmed.slice(2).trim());
    }
  }

  assert(changes.length > 0, `${filePath} 的变更列表不能为空`);

  for (const changeGroup of changes) {
    assert(changeGroup.title.length > 0, `${filePath} 存在空的变更分组标题`);
    assert(changeGroup.items.length > 0, `${filePath} 中分组 ${changeGroup.title} 不能为空`);
  }

  return changes;
}

function parseDownloadChannels(body, filePath, downloadsHeadings) {
  const lines = body.split(/\r?\n/);
  const headingIndex = lines.findIndex((line) => downloadsHeadings.includes(line.trim()));
  assert(headingIndex >= 0, `${filePath} 缺少 ${downloadsHeadings[0]}`);

  const channels = [];

  for (let index = headingIndex + 1; index < lines.length; index += 1) {
    const trimmed = lines[index].trim();
    if (!trimmed) {
      continue;
    }

    if (trimmed.startsWith("## ")) {
      break;
    }

    if (!trimmed.startsWith("- ")) {
      continue;
    }

    const item = trimmed.slice(2).trim();
    const linkMatch = item.match(/^\[(?<label>[^\]]+)\]\((?<url>[^)]+)\)$/);
    if (linkMatch?.groups) {
      channels.push({
        label: linkMatch.groups.label.trim(),
        url: linkMatch.groups.url.trim(),
      });
      continue;
    }

    const plainMatch = item.match(/^(?<label>.+?):\s*(?<url>https?:\/\/\S+)$/);
    assert(plainMatch?.groups, `${filePath} 的下载渠道必须使用 Markdown 链接或“名称: URL”格式`);
    channels.push({
      label: plainMatch.groups.label.trim(),
      url: plainMatch.groups.url.trim(),
    });
  }

  assert(channels.length > 0, `${filePath} 的下载渠道不能为空`);

  for (const channel of channels) {
    assert(channel.label.length > 0, `${filePath} 存在空的下载渠道名称`);
    assert(channel.url.length > 0, `${filePath} 存在空的下载渠道链接`);
  }

  return channels;
}

function releaseAssetUrl(version, fileName) {
  const normalizedVersion = version.trim().replace(/^v/i, "");
  return `https://github.com/${releaseAssetRepo}/releases/download/v${normalizedVersion}/${fileName}`;
}

function buildInstallerDownloads(version) {
  const normalizedVersion = version.trim().replace(/^v/i, "");
  return [
    {
      id: "windows-x64",
      label: "Windows x64",
      url: releaseAssetUrl(normalizedVersion, `locus_${normalizedVersion}_x64-setup.exe`),
      platform: "windows",
      arch: "x64",
      includesManagedPython: true,
      includesManagedGit: true,
      requiresSystemPython: false,
      requiresSystemGit: false,
    },
    {
      id: "windows-x64-without-embed-python-git",
      label: "Windows x64 - system Python/Git",
      url: releaseAssetUrl(normalizedVersion, `locus_${normalizedVersion}_x64-without_embed_python_git-setup.exe`),
      platform: "windows",
      arch: "x64",
      includesManagedPython: false,
      includesManagedGit: false,
      requiresSystemPython: true,
      requiresSystemGit: true,
    },
  ];
}

async function parseLocaleReleaseNotes(docsDir, locale, config) {
  const filePath = path.join(docsDir, config.sourcePath);
  const raw = await readFile(filePath, "utf8");
  const { frontmatter, body } = parseFrontmatter(raw, filePath);

  for (const requiredKey of releaseNoteRequiredFrontmatterKeys) {
    assert(frontmatter[requiredKey], `${filePath} 缺少 frontmatter 字段 ${requiredKey}`);
  }

  return {
    locale,
    filePath,
    frontmatter,
    version: frontmatter.version,
    releasedAt: frontmatter.releasedAt,
    channel: frontmatter.channel,
    localeData: {
      title: frontmatter.updateTitle,
      summary: frontmatter.description,
      changelogUrl: frontmatter.changelogUrl,
      changes: parseChanges(body, filePath, config.changesHeading),
      downloadChannels: parseDownloadChannels(body, filePath, config.downloadsHeadings),
    },
  };
}

async function shouldParseChannelReleaseNotes(docsDir, channel, config) {
  const sourcePaths = Object.values(config.locales).map((localeConfig) =>
    path.join(docsDir, localeConfig.sourcePath),
  );
  const sourceStates = await Promise.all(sourcePaths.map(fileExists));
  const existingCount = sourceStates.filter(Boolean).length;

  if (existingCount === sourceStates.length) {
    return true;
  }

  if (existingCount === 0 && !config.required) {
    return false;
  }

  const missingPaths = sourcePaths
    .filter((_, index) => !sourceStates[index])
    .map((sourcePath) => path.relative(docsDir, sourcePath))
    .join(", ");
  throw new Error(`${channel} release notes source files are incomplete: ${missingPaths}`);
}

export async function parseAllReleaseNotes(docsDir, channel = "stable") {
  const { channel: normalizedChannel, config } = releaseNoteChannelConfig(channel);
  const shouldParse = await shouldParseChannelReleaseNotes(docsDir, normalizedChannel, config);
  assert(shouldParse, `${normalizedChannel} release notes source files are missing`);

  const parsedByLocale = {};

  for (const [locale, localeConfig] of Object.entries(config.locales)) {
    parsedByLocale[locale] = await parseLocaleReleaseNotes(docsDir, locale, localeConfig);
  }

  return parsedByLocale;
}

export async function parseAllChannelReleaseNotes(docsDir) {
  const parsedByChannel = {};

  for (const [channel, config] of Object.entries(releaseNoteChannelConfigs)) {
    const shouldParse = await shouldParseChannelReleaseNotes(docsDir, channel, config);
    if (!shouldParse) {
      continue;
    }

    const parsedByLocale = {};
    for (const [locale, localeConfig] of Object.entries(config.locales)) {
      parsedByLocale[locale] = await parseLocaleReleaseNotes(docsDir, locale, localeConfig);
    }
    parsedByChannel[channel] = parsedByLocale;
  }

  return parsedByChannel;
}

function assertSameFrontmatterShape(reference, target) {
  const referenceKeys = Object.keys(reference.frontmatter).sort();
  const targetKeys = Object.keys(target.frontmatter).sort();

  assert(
    referenceKeys.length === targetKeys.length &&
      referenceKeys.every((key, index) => key === targetKeys[index]),
    `${target.filePath} 的 frontmatter 字段集合必须与 ${reference.filePath} 保持一致`,
  );
}

function validateChannelReleaseNotesMetadata(channel, parsedByLocale) {
  const reference = parsedByLocale.zh;
  assert(reference, "缺少中文 latest-version.mdx，无法作为事实源校验");
  const channelConfig = releaseNoteChannelConfigs[channel];

  for (const [locale, parsed] of Object.entries(parsedByLocale)) {
    const config = channelConfig.locales[locale];
    assertSameFrontmatterShape(reference, parsed);

    assert(
      parsed.frontmatter.channel === channel,
      `${parsed.filePath} 的 channel 必须为 ${channel}`,
    );

    for (const key of releaseNoteSharedFrontmatterKeys) {
      assert(
        parsed.frontmatter[key] === reference.frontmatter[key],
        `${parsed.filePath} 的 ${key} 必须与 ${reference.filePath} 保持一致`,
      );
    }

    assert(
      parsed.frontmatter.changelogUrl === config.expectedChangelogUrl,
      `${parsed.filePath} 的 changelogUrl 必须为 ${config.expectedChangelogUrl}`,
    );

    assert(
      parsed.localeData.downloadChannels.length === reference.localeData.downloadChannels.length,
      `${parsed.filePath} 的下载渠道数量必须与 ${reference.filePath} 保持一致`,
    );

    for (const [index, channel] of parsed.localeData.downloadChannels.entries()) {
      const referenceChannel = reference.localeData.downloadChannels[index];
      assert(
        channel.url === referenceChannel.url,
        `${parsed.filePath} 的下载渠道 ${channel.label} 必须与 ${reference.filePath} 保持相同链接`,
      );
    }
  }

  return parsedByLocale;
}

export async function validateReleaseNotesMetadata(docsDir) {
  const parsedByChannel = await parseAllChannelReleaseNotes(docsDir);

  for (const [channel, parsedByLocale] of Object.entries(parsedByChannel)) {
    validateChannelReleaseNotesMetadata(channel, parsedByLocale);
  }

  return parsedByChannel.stable;
}

function buildUpdateJsonFromParsed(parsedByLocale) {
  const reference = parsedByLocale.zh;
  assert(reference, "缺少中文 latest-version.mdx，无法生成 update manifest");
  const referenceChannel = normalizeReleaseNoteChannel(reference.channel);
  const channelConfig = releaseNoteChannelConfigs[referenceChannel];
  const locales = {};

  for (const [locale, parsed] of Object.entries(parsedByLocale)) {
    for (const key of releaseNoteSharedFrontmatterKeys) {
      assert(
        parsed.frontmatter[key] === reference.frontmatter[key],
        `${channelConfig.locales[locale].sourcePath} 的 ${key} 与中文事实源不一致`,
      );
    }

    locales[locale] = parsed.localeData;
  }

  return {
    version: reference.version,
    releasedAt: reference.releasedAt,
    channel: reference.channel,
    installers: buildInstallerDownloads(reference.version),
    locales,
  };
}

export async function buildUpdateJson(docsDir, channel = "stable") {
  return buildUpdateJsonFromParsed(await parseAllReleaseNotes(docsDir, channel));
}

export async function buildUpdateManifests(docsDir) {
  const parsedByChannel = await parseAllChannelReleaseNotes(docsDir);
  const manifests = {};

  for (const [channel, parsedByLocale] of Object.entries(parsedByChannel)) {
    manifests[channel] = buildUpdateJsonFromParsed(parsedByLocale);
  }

  return manifests;
}
