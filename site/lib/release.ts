// The one place the released version and installer URLs are written.
// Re-pin this on every release (docs/RELEASING.md).
import { gitConfig } from './shared';

export const releaseVersion = '0.1.0-beta.4';
export const releaseTag = `v${releaseVersion}`;

const repoUrl = `https://github.com/${gitConfig.user}/${gitConfig.repo}`;
const downloadBase = `${repoUrl}/releases/download/${releaseTag}`;

export const installCommands = {
  shell: `curl --proto '=https' --tlsv1.2 -LsSf ${downloadBase}/redis-pane-installer.sh | sh`,
  powershell: `irm ${downloadBase}/redis-pane-installer.ps1 | iex`,
} as const;

export const links = {
  repo: repoUrl,
  releases: `${repoUrl}/releases`,
  discussions: `${repoUrl}/discussions`,
  license: `${repoUrl}/blob/${gitConfig.branch}/LICENSE`,
};
