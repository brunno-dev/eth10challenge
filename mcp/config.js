import { fileURLToPath } from 'node:url';
import { writeFile } from 'node:fs/promises';
const serverPath = fileURLToPath(new URL('./server.js', import.meta.url));
const entry = { command: process.execPath, args: [serverPath], env: { ETH_STUDIO_DATA_DIR: `${process.env.LOCALAPPDATA}/dev.ethsearch.studio` } };
const json = JSON.stringify({ mcpServers: { 'eth-search': entry } }, null, 2);
const toml = `[mcp_servers.eth-search]\ncommand = ${JSON.stringify(entry.command)}\nargs = ${JSON.stringify(entry.args)}\nstartup_timeout_sec = 20\ntool_timeout_sec = 150\n\n[mcp_servers.eth-search.env]\nETH_STUDIO_DATA_DIR = ${JSON.stringify(entry.env.ETH_STUDIO_DATA_DIR)}\n`;
if (process.argv.includes('--write')) {
  await writeFile(new URL('./claude-config.local.json', import.meta.url), json+'\n');
  await writeFile(new URL('./codex-config.local.toml', import.meta.url), toml);
  console.log('Criados claude-config.local.json e codex-config.local.toml. Mescle nos arquivos de configuração dos clientes; veja README.md.');
} else console.log(process.argv.includes('--codex') ? toml : json);
