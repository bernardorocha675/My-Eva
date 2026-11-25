// Central: liga e desliga a Eva, mostra as novidades (guardadas no programa) e os ajustes.
const temTauri = !!window.__TAURI__;
const DEMO = { ligada: false, versao: "1.1.3", inicia: true, perguntar: false };
const invoke = temTauri ? window.__TAURI__.core.invoke : async (cmd) => (cmd === "central_estado" ? DEMO : null);
const listen = temTauri ? window.__TAURI__.event.listen : async () => {};

if (/Linux/.test(navigator.userAgent) && !/Android/.test(navigator.userAgent)) {
  document.documentElement.classList.add("linux");
  setInterval(() => {
    const m = document.querySelector("#ct-mascote .mascote");
    if (!m || document.hidden) return;
    m.classList.add("piscando");
    setTimeout(() => m.classList.remove("piscando"), 150);
  }, 8000);
}
const el = (id) => document.getElementById(id);
const central = el("central");
const botao = el("ct-ligar");
const pergunta = el("ct-pergunta");
let ligada = false;
let ocupada = false;
// Atualização: nada aqui usa a internet se não existir endereço ou se estiver desligada.
let atu = { existe: false, ligada: false, sozinha: false };
let nova = null; // { versao, notas } quando há versão nova
let aviso = null; // [título, subtítulo, humor] no lugar do "Tudo pronto"

// ---------------- Mascote (o mesmo desenho do painel) ----------------
async function trazerMascote() {
  const html = await (await fetch("index.html")).text();
  const original = new DOMParser().parseFromString(html, "text/html").querySelector(".mascote");
  const por = (destino, humor, prefixo) => {
    const m = original.cloneNode(true);
    m.removeAttribute("id");
    m.dataset.humor = humor;
    // Cada cópia precisa dos seus próprios recortes (ids não podem repetir na página).
    m.innerHTML = m.innerHTML
      .replace(/id="(olho-e|olho-d|lentes)"/g, `id="${prefixo}$1"`)
      .replace(/url\(#(olho-e|olho-d|lentes)\)/g, `url(#${prefixo}$1)`);
    el(destino).replaceChildren(m);
  };
  por("ct-mascote", "deboche", "ct-");
  por("cp-mascote", "seria", "cp-");
  desenhar();
}
function humor(qual) {
  const m = el("ct-mascote").firstElementChild;
  if (m) m.dataset.humor = qual;
}

// ---------------- Estado ----------------
function desenhar() {
  const [titulo, sub, cara] = aviso || (ligada
    ? ["Já estou ligada", "Olha eu no topo da tela", "piscadinha"]
    : ["Tudo pronto", "Quando quiser, é só me ligar", "deboche"]);
  el("ct-titulo").textContent = titulo;
  el("ct-sub").textContent = sub;
  botao.textContent = ligada ? "Desligar a Eva" : "Ligar a Eva";
  botao.classList.toggle("desligar", ligada);
  botao.disabled = ocupada;
  humor(cara);
  const instalar = el("ct-instalar");
  instalar.hidden = !nova;
  instalar.disabled = ocupada;
  if (nova) instalar.textContent = ligada ? "Desligar e atualizar" : `Instalar a ${nova.versao}`;
}

async function atualizar() {
  const e = await invoke("central_estado");
  if (!e) return;
  // No meio do ligar/desligar, quem manda na tela é a própria ação.
  ligada = e.ligada;
  el("ct-versao").textContent = `Versão ${e.versao}`;
  el("ct-inicio").checked = e.inicia;
  versaoAtual = e.versao;
  mostrarNovidades();
  if (e.perguntar) perguntar();
  if (!ligada) pergunta.hidden = true;
  if (!ocupada) desenhar();
}

function falhou(titulo) {
  ocupada = false;
  central.classList.remove("saindo");
  desenhar();
  el("ct-titulo").textContent = titulo;
  el("ct-sub").textContent = "Tenta de novo?";
  humor("seria");
}

const esperar = (ms) => new Promise((r) => setTimeout(r, ms));

async function ligarEva() {
  ocupada = true;
  desenhar();
  el("ct-titulo").textContent = "Até já";
  el("ct-sub").textContent = "Indo pro topo da tela";
  humor("piscadinha");
  await esperar(350);
  central.classList.add("saindo");
  await esperar(420);
  try {
    await invoke("central_ligar"); // a Central é fechada pelo Rust
  } catch (e) {
    // "ocupada": a Eva anterior ainda está terminando de sair.
    if (e === "ocupada") {
      await esperar(1200);
      try { await invoke("central_ligar"); return; } catch (e2) { /* cai no aviso */ }
    }
    falhou("Não consegui ligar");
  }
}

async function desligarEva(confirmado, depois) {
  ocupada = true;
  pergunta.hidden = true;
  botao.disabled = true;
  try {
    const desligou = await invoke("central_desligar", { confirmado });
    // Se perguntou antes, guarda o que fazer caso a resposta seja desligar.
    depoisDeDesligar = desligou ? null : depois;
  } catch (e) {
    return falhou("Não consegui desligar");
  }
  ocupada = false;
  if (nova && aviso) aviso = null;
  await atualizar();
  if (depois && !ligada) depois();
}

function perguntar() {
  pergunta.hidden = false;
  el("cp-ver").focus();
}

botao.addEventListener("click", () => {
  if (ocupada) return;
  if (ligada) desligarEva(false);
  else ligarEva();
});
el("cp-ver").addEventListener("click", () => {
  depoisDeDesligar = null;
  pergunta.hidden = true;
  invoke("central_ver_pedido");
});
el("cp-desligar").addEventListener("click", () => desligarEva(true, depoisDeDesligar));
el("ct-fechar").addEventListener("click", () => invoke("central_fechar"));
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !pergunta.hidden) {
    pergunta.hidden = true;
    depoisDeDesligar = null;
  }
});
el("ct-inicio").addEventListener("change", async (e) => {
  e.target.checked = !!(await invoke("central_inicio", { ligar: e.target.checked }));
});

// ---------------- Atualização ----------------
let depoisDeDesligar = null;
let versaoAtual = "";

function desenharAjustes() {
  el("ct-aj-atualizar").hidden = !atu.existe;
  el("ct-atualizar-auto").checked = atu.ligada;
  el("ct-nota-rede").hidden = !atu.existe || atu.ligada;
}

async function iniciarAtualizacao() {
  atu = (await invoke("atualizacao_estado")) || atu;
  desenharAjustes();
  if (atu.existe && atu.ligada) procurar();
}

const ERROS = {
  rede: ["Sem internet", "Dá pra me ligar na versão de agora", "neutra"],
  assinatura: ["Descartei a atualização", "Não confiei na assinatura. Continuo como estava", "seria"],
};

async function procurar() {
  if (!ligada) {
    aviso = ["Procurando novidades", "Já pode me ligar, se quiser", "neutra"];
    if (!ocupada) desenhar();
  }
  let achou = null;
  let erro = null;
  try {
    achou = await invoke("atualizacao_procurar");
  } catch (e) {
    erro = e;
  }
  if (!atu.ligada) achou = erro = null; // desligou o ajuste enquanto procurava
  nova = achou;
  aviso = erro && !ligada ? ERROS[erro] || ["Não consegui procurar", "Dá pra me ligar normalmente", "neutra"] : null;
  mostrarNovidades();
  if (ocupada) return; // já está ligando: fica pra próxima
  if (nova && ligada) aviso = [`Saiu a ${nova.versao}`, "Instalo quando você me desligar", "deboche"];
  else if (nova && !atu.sozinha) aviso = [`Saiu a ${nova.versao}`, "O sistema vai pedir sua senha", "deboche"];
  desenhar();
  if (nova && !ligada && atu.sozinha) instalar();
}

async function instalar() {
  if (ocupada || !nova) return;
  ocupada = true;
  aviso = [`Baixando a ${nova.versao}`, "0% baixado", "neutra"];
  desenhar();
  botao.textContent = "Atualizando…";
  el("ct-fechar").disabled = true;
  el("ct-nivel").style.width = "0";
  el("ct-barra").hidden = false;
  try {
    await invoke("atualizacao_instalar"); // reabre o programa na versão nova
  } catch (e) {
    ocupada = false;
    el("ct-fechar").disabled = false;
    el("ct-barra").hidden = true;
    if (e === "assinatura") nova = null; // não oferece de novo um arquivo ruim
    aviso = ERROS[e] || ["Não consegui atualizar", "Continuo na versão de agora", "seria"];
    mostrarNovidades();
    await atualizar();
  }
}
listen("atualizacao-progresso", (e) => {
  el("ct-sub").textContent = `${e.payload}% baixado`;
  el("ct-nivel").style.width = `${e.payload}%`;
});
listen("atualizacao-instalando", () => {
  el("ct-titulo").textContent = "Instalando";
  el("ct-sub").textContent = "Assinatura conferida. Já volto";
});
el("ct-instalar").addEventListener("click", () => {
  if (ocupada) return;
  if (ligada) desligarEva(false, instalar);
  else instalar();
});
el("ct-atualizar-auto").addEventListener("change", async (e) => {
  atu.ligada = !!(await invoke("atualizacao_definir", { ligar: e.target.checked }));
  desenharAjustes();
  if (atu.ligada) return procurar();
  nova = null;
  aviso = null;
  mostrarNovidades();
  if (!ocupada) desenhar();
});

// ---------------- Abas ----------------
const abas = [...document.querySelectorAll(".ct-abas button")];
abas.forEach((b) => b.addEventListener("click", () => {
  abas.forEach((o) => {
    o.setAttribute("aria-pressed", String(o === b));
    el(`aba-${o.dataset.aba}`).hidden = o !== b;
  });
}));

function item(rotulo, texto, classe) {
  const d = document.createElement("div");
  d.className = "ct-item";
  const b = document.createElement("b");
  b.className = classe;
  b.textContent = rotulo;
  const s = document.createElement("span");
  s.textContent = texto;
  d.append(b, s);
  return d;
}
// As notas da versão nova vêm em linhas "Novo: texto", no mesmo arquivo que anuncia a versão.
function itensDasNotas(notas) {
  return notas.split("\n").map((l) => l.trim()).filter(Boolean).slice(0, 12).map((l) => {
    const m = l.match(/^(Novo|Melhor|Corrigido):\s*(.+)$/);
    return m ? [m[1], m[2]] : ["", l];
  });
}
function mostrarNovidades() {
  const atual = nova
    ? { versao: nova.versao, itens: itensDasNotas(nova.notas) }
    : NOVIDADES.find((v) => v.versao === versaoAtual) || NOVIDADES[0];
  const titulo = document.createElement("span");
  titulo.className = "ct-sobre";
  titulo.textContent = `O que ${nova ? "vem" : "tem"} na ${atual.versao}`;
  el("aba-novidades").replaceChildren(titulo, ...atual.itens.map(([t, x]) => item(t, x, t === "Novo" ? "novo" : "")));
  el("aba-versoes").replaceChildren(...NOVIDADES.map((v) => item(v.versao, v.resumo, "versao")));
}

listen("central-mudou", atualizar);
trazerMascote().catch(() => {});
atualizar().then(iniciarAtualizacao);
