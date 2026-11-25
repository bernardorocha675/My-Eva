// Monta o latest.json que a Central lê: versão, novidades, endereços e assinaturas.
// Uso: node manifesto.mjs <pasta dos instaladores> <versão> <repositório público>
import { readFileSync, writeFileSync } from "node:fs";
import vm from "node:vm";

const [pasta, versao, repo] = process.argv.slice(2);
const janela = {};
vm.runInNewContext(readFileSync("src/novidades.js", "utf8"), { window: janela });
const v = janela.NOVIDADES.find((n) => n.versao === versao);
if (!v) throw new Error(`src/novidades.js não tem a versão ${versao}`);
const notas = v.itens.length ? v.itens.map(([t, x]) => `${t}: ${x}`).join("\n") : v.resumo;

const plataforma = (arquivo) => ({
  url: `https://github.com/${repo}/releases/download/v${versao}/${arquivo}`,
  signature: readFileSync(`${pasta}/${arquivo}.sig`, "utf8"),
});
const linux = `my-eva_${versao}_amd64.AppImage`;
const windows = `my-eva_${versao}_x64-setup.exe`;
writeFileSync(`${pasta}/latest.json`, JSON.stringify({
  version: versao,
  notes: notas,
  pub_date: new Date().toISOString(),
  platforms: { "linux-x86_64": plataforma(linux), "windows-x86_64": plataforma(windows) },
}, null, 2));
