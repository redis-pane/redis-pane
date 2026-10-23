// The landing page's version badge. Bump it on every release (docs/RELEASING.md). The installer
// URLs use /releases/latest and never change; `latest` is the newest non-prerelease.
import { gitConfig } from './shared';

export const releaseVersion = '1.0.0';

const repoUrl = `https://github.com/${gitConfig.user}/${gitConfig.repo}`;
const downloadBase = `${repoUrl}/releases/latest/download`;

export const installCommands = {
  homebrew: 'brew install redis-pane/tap/redis-pane',
  shell: `curl --proto '=https' --tlsv1.2 -LsSf ${downloadBase}/redis-pane-installer.sh | sh`,
  powershell: `irm ${downloadBase}/redis-pane-installer.ps1 | iex`,
} as const;

export const links = {
  repo: repoUrl,
  releases: `${repoUrl}/releases`,
  discussions: `${repoUrl}/discussions`,
  windowsIssue: `${repoUrl}/issues/138`,
  story: 'https://vinodsantharam.github.io/blog/the-refresh-button-that-didnt-refresh/',
  license: `${repoUrl}/blob/${gitConfig.branch}/LICENSE`,
};
