// Eva: mascote + painel no topo da tela.
const temTauri = !!window.__TAURI__;
if (/Linux/.test(navigator.userAgent) && !/Android/.test(navigator.userAgent)) document.documentElement.classList.add("linux");
const DEMO = { conectado: true, bateria: 75, carregando: false, conexao: "bluetooth" };
const DEMO_MUSICA = {
  ativo: true, titulo: "Nome da música", artista: "Artista", app: "Spotify",
  tocando: true, posicao: 72, duracao: 225,
};
const invoke = temTauri
  ? window.__TAURI__.core.invoke
  : async (cmd) => ({ ds4_status: DEMO, musica_status: [DEMO_MUSICA, null] })[cmd] ?? null;
const listen = temTauri ? window.__TAURI__.event.listen : async () => {};

const eva = document.getElementById("eva");
const mascote = document.getElementById("mascote");
const trilho = document.getElementById("trilho");
const pontos = document.getElementById("pontos");
const secControle = document.getElementById("sec-controle");
const pct = document.getElementById("pct");
const sub = document.getElementById("sub");

const TEMPO_SUMIR = 5000;
const BATERIA_FRACA = 20;
// Tamanho da área "viva" de cada modo (o resto da janela deixa o clique passar).
const AREAS = { inicio: [120, 124], intro: [120, 124], nome: [300, 124], aberta: [420, 112], oculta: [160, 8] };

let modoAtual = "inicio";
let mouseDentro = false;
let timerSumir = null;
let timerHumor = null;
let secaoAtual = 0;
const secoes = [...trilho.children];

// ---------------- Modos: intro, aberta, oculta ----------------
// "expandida" (responder / escolher opções) e "aprovacao" têm o tamanho medido pelo conteúdo.
const MODOS = ["inicio", "intro", "nome", "aberta", "oculta", "expandida", "aprovacao"];
const expandido = (m = modoAtual) => m === "expandida" || m === "aprovacao";

function definirArea(w, h) {
  const area = { x: (window.innerWidth - w) / 2, y: 0, w, h };
  invoke("definir_area", { area }).catch(() => {});
}

function definirModo(modo) {
  modoAtual = modo;
  eva.classList.remove(...MODOS);
  eva.classList.add(modo);
  if (expandido(modo)) return; // claudeAjustarTamanho() cuida do tamanho e da área
  eva.style.width = "";
  eva.style.height = "";
  const [w, h] = AREAS[modo];
  definirArea(w, h);
}

let timerLetreiro = null;
function abrir(secao) {
  // Se tem um pedido do Claude esperando resposta, abrir volta direto pra ele.
  if (claudeReabrir()) return;
  if (secao != null) irPara(secao);
  const jaAberta = modoAtual === "aberta";
  definirModo("aberta");
  agendarSumir();
  atualizarGeral(); // relógio e chips em dia assim que aparece
  desenharTempo();
  // mede o letreiro da música quando o painel terminar de abrir
  if (!jaAberta) {
    clearTimeout(timerLetreiro);
    timerLetreiro = setTimeout(() => definirTitulo(musTituloAtual), 750);
  }
}

function esconder() {
  clearTimeout(timerSumir);
  claudeAoEsconder();
  definirModo("oculta");
}

function agendarSumir() {
  clearTimeout(timerSumir);
  // Expandida (respondendo/aprovando) nunca some sozinha.
  if (!mouseDentro && !expandido()) timerSumir = setTimeout(esconder, TEMPO_SUMIR);
}

function alternar() {
  if (modoAtual === "aberta" || expandido()) esconder();
  else if (modoAtual === "oculta") abrir();
}

// ---------------- Humor da mascote ----------------
// humorBase: a cara "de fundo" (curtindo a música, séria se pegou no flagra...).
// humor(qual, ms): uma reação passageira; depois de ms volta pro humorBase.
let humorBase = "neutra";
let introFeita = false;

function humor(qual, ms) {
  clearTimeout(timerHumor);
  timerHumor = null;
  mascote.dataset.humor = qual;
  if (ms) timerHumor = setTimeout(() => { timerHumor = null; aplicarHumorBase(); }, ms);
}

// humorFixo: enquanto um pedido sério do Claude está na tela, a cara não muda.
let humorFixo = null;

function aplicarHumorBase() {
  // Durante a intro e enquanto uma reação passageira roda, não mexe.
  if (!introFeita || timerHumor) return;
  mascote.dataset.humor = humorFixo || humorBase;
}

// ---------------- Seções deslizantes ----------------
secoes.forEach((_, i) => {
  const p = document.createElement("span");
  p.className = "ponto";
  p.addEventListener("click", () => irPara(i));
  pontos.appendChild(p);
});

function irPara(i) {
  secaoAtual = (i + secoes.length) % secoes.length;
  trilho.style.transform = `translateX(-${secaoAtual * 100}%)`;
  [...pontos.children].forEach((p, j) => p.classList.toggle("ativo", j === secaoAtual));
  eva.classList.toggle("na-musica", secoes[secaoAtual].id === "sec-musica");
}

document.getElementById("anterior").addEventListener("click", () => irPara(secaoAtual - 1));
document.getElementById("proxima").addEventListener("click", () => irPara(secaoAtual + 1));
let travaRoda = false;
eva.addEventListener("wheel", (e) => {
  if (modoAtual !== "aberta" || travaRoda) return;
  travaRoda = true;
  setTimeout(() => (travaRoda = false), 350);
  irPara(secaoAtual + (e.deltaY > 0 || e.deltaX > 0 ? 1 : -1));
});

// ---------------- Mouse e tecla INS ----------------
function aoMouse(dentro) {
  mouseDentro = dentro;
  if (dentro) {
    clearTimeout(timerSumir);
    if (modoAtual === "oculta") abrir();
    claudeAoMouseEntrar();
  } else if (modoAtual === "aberta") {
    agendarSumir();
  }
}
// O Rust vigia o cursor (mesmo com a janela deixando o clique passar).
listen("eva-mouse", (e) => aoMouse(e.payload));
listen("eva-atalho", alternar);
// Central: mostrar o pedido que está esperando, ou se despedir (sobe e some).
listen("eva-abrir", () => introFeita && abrir());
listen("eva-despedir", () => {
  clearTimeout(timerSumir);
  introFeita = false;
  definirModo("inicio");
});

// ---------------- Controle (DS4) ----------------
let ds4 = { conectado: false, bateria: 0, carregando: false, conexao: "" };
const fraca = (s) => s.conectado && !s.carregando && s.bateria <= BATERIA_FRACA;

function desenharControle(s) {
  secControle.classList.toggle("desligado", !s.conectado);
  if (!s.conectado) return;
  secControle.style.setProperty("--nivel", s.bateria);
  pct.textContent = `${s.bateria}%`;
  secControle.classList.toggle("carregando", s.carregando);
  secControle.classList.toggle("fraca", s.bateria <= BATERIA_FRACA);
  secControle.classList.toggle("media", s.bateria > BATERIA_FRACA && s.bateria <= 50);
  const via = s.conexao === "bluetooth" ? "Bluetooth" : "USB";
  if (s.carregando) sub.textContent = s.bateria >= 100 ? "Carga completa" : "Carregando…";
  else if (fraca(s)) sub.textContent = "Bateria fraca, carrega isso aí";
  else sub.textContent = `Conectado via ${via}`;
}

function avisar() {
  // Durante a intro não interrompe; depois, mostra a seção do controle.
  if (!introFeita) return;
  abrir(secoes.findIndex((s) => s.id === "sec-controle"));
}

function aplicarDs4(novo) {
  const antes = ds4;
  ds4 = novo;
  desenharControle(novo);
  // Durante a entrada ela não reage (senão debocha no meio do revirar de olhos).
  if (!introFeita) return;

  if (novo.conectado && !antes.conectado) {
    humor("deboche", 3500);
    avisar();
  } else if (!novo.conectado && antes.conectado) {
    humor("seria", 2500);
    avisar();
  } else if (fraca(novo) && !fraca(antes)) {
    humor("seria", 6000);
    avisar();
  } else if (novo.carregando !== antes.carregando) {
    avisar();
  }
}

listen("ds4-status", (e) => aplicarDs4(e.payload));

// ---------------- Música ----------------
const SECAO_MUSICA = secoes.findIndex((s) => s.id === "sec-musica");
const secMusica = document.getElementById("sec-musica");
const musCapa = document.getElementById("mus-capa");
const musCapaImg = document.getElementById("mus-capa-img");
const musLetreiro = document.getElementById("mus-letreiro");
const musArtista = document.getElementById("mus-artista");
const musPlay = document.getElementById("mus-play");
const musTempo = document.getElementById("mus-tempo");
const musProgresso = document.getElementById("mus-progresso");

let musica = { ativo: false, titulo: "", artista: "", app: "", tocando: false, posicao: 0, duracao: 0 };
let musPos = 0;                  // posição local, andando entre as atualizações do Rust
let musUltimoTique = performance.now();

const mmss = (s) => {
  s = Math.max(0, Math.floor(s));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
};

// Nome que não cabe rola devagar pro lado (letreiro) em vez de cortar com "…".
// Só dá pra medir com o painel aberto; abrir() chama de novo depois da animação.
let musTituloAtual = "Nada tocando";
function definirTitulo(texto) {
  musTituloAtual = texto;
  musLetreiro.classList.remove("rolando");
  musLetreiro.textContent = "";
  const a = document.createElement("span");
  a.textContent = texto;
  musLetreiro.appendChild(a);
  requestAnimationFrame(() => {
    if (modoAtual !== "aberta" || musLetreiro.clientWidth === 0) return;
    if (a.offsetWidth <= musLetreiro.clientWidth + 2) return;
    const faixa = document.createElement("span");
    faixa.className = "faixa";
    const b = a.cloneNode(true);
    b.setAttribute("aria-hidden", "true");
    musLetreiro.textContent = "";
    faixa.append(a, b);
    musLetreiro.appendChild(faixa);
    musLetreiro.classList.add("rolando");
  });
}

function desenharTempo() {
  if (!musica.ativo || musica.duracao <= 0) {
    musTempo.textContent = musica.ativo ? musica.app : "";
    musProgresso.style.setProperty("--prog", 0);
    return;
  }
  musTempo.textContent = `${mmss(musPos)} / ${mmss(musica.duracao)}`;
  musProgresso.style.setProperty("--prog", Math.min(100, (musPos / musica.duracao) * 100));
}

function aplicarCapa(url) {
  if (url) musCapaImg.src = url;
  musCapa.classList.toggle("com-imagem", !!url);
}

function aplicarMusica(novo) {
  const antes = musica;
  musica = novo;
  musPos = novo.posicao;
  musUltimoTique = performance.now();

  secMusica.classList.toggle("vazia", !novo.ativo);
  eva.classList.toggle("tem-musica", novo.ativo);
  eva.classList.toggle("pausada", novo.ativo && !novo.tocando);
  musPlay.classList.toggle("tocando", novo.tocando);
  musPlay.setAttribute("aria-label", novo.tocando ? "Pausar" : "Tocar");

  const trocouFaixa = novo.titulo !== antes.titulo || novo.artista !== antes.artista;
  if (trocouFaixa) {
    definirTitulo(novo.ativo ? novo.titulo : "Nada tocando");
    musArtista.textContent = novo.ativo ? (novo.artista || novo.app) : "Abra o Spotify ou um vídeo";
    if (!novo.ativo) aplicarCapa(null);
    // Música nova começou: mostra rapidinho o que está tocando.
    if (novo.ativo && antes.ativo && modoAtual === "oculta") abrir(SECAO_MUSICA);
  }

  // Cara de fundo: curtindo enquanto toca; pausou no meio? pega no flagra, fica séria.
  if (novo.ativo && novo.tocando) humorBase = "curtindo";
  else if (novo.ativo && antes.tocando && !novo.tocando) humorBase = "seria";
  else if (!novo.ativo) humorBase = "neutra";
  aplicarHumorBase();

  desenharTempo();
}

// A barra anda sozinha entre as atualizações (que chegam 1x por segundo).
setInterval(() => {
  const agora = performance.now();
  if (musica.tocando) musPos += (agora - musUltimoTique) / 1000;
  musUltimoTique = agora;
  if (musica.duracao > 0) musPos = Math.min(musPos, musica.duracao);
  if (modoAtual !== "oculta") desenharTempo(); // escondida não precisa redesenhar
}, 250);

function comandoMusica(acao) {
  invoke("musica_acao", { acao }).catch(() => {});
  // Resposta imediata no play/pause (o Windows confirma logo depois).
  if (acao === "tocar_pausar" && musica.ativo) aplicarMusica({ ...musica, posicao: musPos, tocando: !musica.tocando });
}
musPlay.addEventListener("click", () => comandoMusica("tocar_pausar"));
document.getElementById("mus-proxima").addEventListener("click", () => comandoMusica("proxima"));
document.getElementById("mus-anterior").addEventListener("click", () => comandoMusica("anterior"));

listen("musica", (e) => aplicarMusica(e.payload));
listen("musica-capa", (e) => aplicarCapa(e.payload));

// ---------------- Claude: avisos, responder, aprovar ----------------
// O Rust recebe os recados do Claude Code (hooks) e manda:
//   "claude-aviso"   → só um aviso ("tô esperando"), sem resposta.
//   "claude-pedido"  → algo que espera você: terminei / permissao / pergunta.
//   "claude-expirou" → o pedido venceu sem resposta (o Claude seguiu normal).
const SECAO_CLAUDE = secoes.findIndex((s) => s.id === "sec-claude");
const secClaude = document.getElementById("sec-claude");
const clSelo = document.getElementById("cl-selo");
const clIcone = document.getElementById("cl-icone");
const clIconePath = document.getElementById("cl-icone-path");
const clTitulo = document.getElementById("cl-titulo");
const clSub = document.getElementById("cl-sub");
const clBotoes = document.getElementById("cl-botoes");
const clExp = document.getElementById("cl-expandido");

const LARGURAS = { expandida: 540, aprovacao: 640 };
const ICONES = {
  ok: "M5 12.5l4.5 4.5L19 7.5",
  relogio: "M12 4a8 8 0 1 0 0 16 8 8 0 0 0 0-16zM12 8v4.5l3 2",
  escudo: "M12 3l7 3v5c0 4.5-3 8-7 10-4-2-7-5.5-7-10V6z",
  pergunta: "M9.5 9a2.5 2.5 0 1 1 3.5 2.3c-.6.3-1 .9-1 1.6V14M12 17.5v.5",
  x: "M6 6l12 12M18 6L6 18",
  descanso: "M7 12h10",
};

let pedido = null;        // pedido do Claude sendo mostrado agora
let pedidoMostradoEm = 0; // quando ele apareceu (trava de clique duplo)
const fila = [];          // pedidos que chegaram enquanto outro estava na tela
let timerResultado = null;

const esc = (t) => String(t ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const recorte = (t, n) => { t = String(t || "").replace(/\s+/g, " ").trim(); return t.length > n ? t.slice(0, n - 1) + "…" : t; };

// --- aviso pendente (brilho apressado na alcinha) ---
function marcarPendente(cor) { eva.dataset.pendente = cor; }
function marcarVisto() { delete eva.dataset.pendente; }

// --- seção compacta ---
function desenharCompacto({ selo, cor, icone, titulo, sub, botoes = [] }) {
  secClaude.classList.toggle("vazio", !cor);
  clSelo.textContent = selo;
  clSelo.className = "cl-selo" + (cor ? " " + cor : "");
  clIcone.className = "cl-icone" + (cor ? " " + cor : "");
  clIconePath.setAttribute("d", ICONES[icone] || ICONES.ok);
  clTitulo.textContent = titulo;
  clSub.textContent = sub;
  clBotoes.innerHTML = botoes
    .map((b) => `<button class="cl-btn ${b.estilo}" data-acao="${b.acao}">${esc(b.texto)}</button>`)
    .join("");
}

function compactoVazio() {
  desenharCompacto({ selo: "Tudo certo", icone: "descanso", titulo: "Nenhum aviso", sub: "Te chamo quando o Claude precisar de você" });
}

function mostrarResultado(titulo, sub, cor, icone) {
  desenharCompacto({ selo: cor === "vermelho" ? "Negado" : "Pronto", cor, icone, titulo, sub });
  clearTimeout(timerResultado);
  timerResultado = setTimeout(() => { if (!pedido) compactoVazio(); }, 5000);
}

// --- tamanho da janela expandida (medido pelo conteúdo) ---
function claudeAjustarTamanho() {
  if (!expandido()) return;
  const w = LARGURAS[modoAtual];
  clExp.style.width = w + "px";
  const h = Math.min(clExp.offsetHeight, window.innerHeight - 10);
  eva.style.width = w + "px";
  eva.style.height = h + "px";
  definirArea(w, h);
}

function expandir(modo, html) {
  clearTimeout(timerSumir);
  clExp.innerHTML = html;
  irPara(SECAO_CLAUDE);
  definirModo(modo);
  claudeAjustarTamanho();
  setTimeout(() => clExp.querySelector("textarea, .cl-campo")?.focus(), 400);
}

function recolher() {
  definirModo("aberta");
  irPara(SECAO_CLAUDE);
  agendarSumir();
}

// --- conteúdo de cada janela ---
const cabecalho = (titulo, sub, selo, cor, fechar) => `
  <div class="cl-cab">
    <div class="cl-cab-txt"><b>${esc(titulo)}</b><span>${esc(sub)}</span></div>
    <span class="cl-selo ${cor}">${esc(selo)}</span>
    ${fechar ? `<button class="cl-fechar" data-acao="recolher" aria-label="Recolher"><svg viewBox="0 0 24 24"><path d="M6 15l6-6 6 6"/></svg></button>` : ""}
  </div>`;

function htmlResponder(p) {
  const rapidas = ["Pode seguir", "Explica melhor", "Revisa o que fez", "Para por aqui"];
  return `
    ${cabecalho(`Claude · ${p.projeto}`, "Respondendo pela Eva", "Terminei", "verde", true)}
    <div class="cl-mensagem">${esc(p.mensagem || "Terminei a tarefa.")}</div>
    <div class="cl-rapidas">${rapidas.map((r) => `<button class="cl-rapida" data-acao="rapida">${esc(r)}</button>`).join("")}</div>
    <div class="cl-escrever">
      <textarea id="cl-texto" rows="2" placeholder="Escreva sua resposta…" aria-label="Sua resposta pro Claude"></textarea>
      <button class="cl-enviar" data-acao="enviar" aria-label="Enviar pro Claude"><svg viewBox="0 0 24 24"><path d="M12 19V5M6 11l6-6 6 6"/></svg></button>
    </div>
    <span class="cl-dica">Enter envia · Shift + Enter quebra linha · Esc recolhe</span>`;
}

function htmlAprovacao(p) {
  const riscos = { baixo: ["Baixo", 1], medio: ["Médio", 2], alto: ["Alto", 3] };
  const [nomeRisco, barras] = riscos[p.risco] || riscos.medio;
  const alto = p.risco === "alto";
  return `
    ${cabecalho("O Claude quer sua permissão", `${p.projeto} · ${p.cwd || ""}`, "Esperando você", "laranja", false)}
    <div class="cl-bloco"><span class="cl-legenda">Resumo do pedido</span><span class="cl-resumo">${esc(p.resumo)}</span></div>
    ${p.comando ? `<div class="cl-comando"><span>${esc(p.ferramenta)}</span><code>${esc(p.comando)}</code></div>` : ""}
    ${p.detalhes ? `<div class="cl-regra"><span class="cl-legenda">${p.comando ? "Outros detalhes do pedido" : "Tudo o que vai ser enviado"}</span><pre class="cl-detalhes">${esc(p.detalhes)}</pre></div>` : ""}
    ${p.nota ? `<div class="cl-bloco"><span class="cl-legenda">O Claude descreve como</span><p class="cl-porque cl-nota">“${esc(p.nota)}”</p></div>` : ""}
    ${p.porque ?`<div class="cl-bloco"><span class="cl-legenda">Por quê</span><p class="cl-porque">${esc(p.porque)}</p></div>` : ""}
    <div class="cl-duas">
      <div class="cl-caixa"><span class="cl-legenda">O que vai acontecer</span>
        ${(p.efeitos || []).map((e) => `<div class="cl-efeito">• ${esc(e)}</div>`).join("") || `<div class="cl-efeito">• Usa a ferramenta ${esc(p.ferramenta)}</div>`}
      </div>
      <div class="cl-risco ${esc(p.risco)}"><span class="cl-legenda">Risco</span>
        <div class="cl-barras">${[1, 2, 3].map((i) => `<i class="${i <= barras ? "on" : ""}"></i>`).join("")}<b>${nomeRisco}</b></div>
        <span>${esc(p.riscoTexto)}</span>
      </div>
    </div>
    ${p.podeSempre && p.regra ? `<div class="cl-regra"><span class="cl-legenda">“Sempre permitir” grava esta regra</span><code>${esc(p.regra)}</code></div>` : ""}
    <div class="cl-motivo" hidden>
      <input class="cl-campo" id="cl-motivo" placeholder="Diga ao Claude por que não (opcional)" aria-label="Motivo pra negar">
      <button class="cl-btn perigo" data-acao="negar-confirmar">Negar</button>
    </div>
    <div class="cl-acoes">
      <button class="cl-btn contorno-perigo" data-acao="negar">Negar…</button>
      ${p.podeSempre ? `<button class="cl-btn secundario" data-acao="sempre">Sempre permitir esta regra</button>` : ""}
      <button class="cl-btn ${alto ? "perigo" : "primario"}" data-acao="permitir">${alto ? "Permitir mesmo assim" : "Permitir uma vez"}</button>
    </div>
    <button class="cl-link" data-acao="no-claude">Responder no próprio Claude</button>`;
}

function htmlPergunta(p) {
  const blocos = (p.perguntas || []).map((q, qi) => `
    <div class="cl-bloco">
      <span class="cl-pergunta">${esc(q.question)}</span>
      <div class="cl-opcoes">
        ${(q.options || []).map((o, oi) => `
          <button class="cl-opcao ${q.multiSelect ? "multi" : ""}" data-acao="opcao" data-q="${qi}" data-o="${oi}">
            <span class="marca"></span><span><b>${esc(o.label)}</b>${o.description ? `<small>${esc(o.description)}</small>` : ""}</span>
          </button>`).join("")}
      </div>
      <input class="cl-campo" data-outra="${qi}" placeholder="Ou escreva outra resposta…" aria-label="Outra resposta">
    </div>`).join("");
  return `
    ${cabecalho(`Claude · ${p.projeto}`, "Tem uma pergunta pra você", "Pergunta", "roxo", false)}
    ${blocos}
    <div class="cl-acoes"><button class="cl-btn primario" data-acao="responder-pergunta">Responder</button></div>
    <button class="cl-link" data-acao="no-claude">Responder no próprio Claude</button>`;
}

// --- chegada de avisos e pedidos ---
function claudeAviso(a) {
  if (pedido) return; // um pedido de verdade tem prioridade
  desenharCompacto({
    selo: "Esperando você", cor: "amarelo", icone: "relogio",
    titulo: "Tô esperando sua resposta", sub: `${a.projeto} · responda lá no Claude`,
    botoes: [{ texto: "Ok", estilo: "secundario", acao: "ok-aviso" }],
  });
  marcarPendente("amarelo");
  if (modoAtual === "oculta" || modoAtual === "aberta") abrir(SECAO_CLAUDE);
}

function claudePedido(p) {
  if (pedido) { fila.push(p); return; }
  pedido = p;
  pedidoMostradoEm = Date.now();
  clearTimeout(timerResultado);
  if (p.tipo === "terminei") {
    desenharCompacto({
      selo: "Concluído", cor: "verde", icone: "ok",
      titulo: "Terminei a tarefa", sub: `${p.projeto} · ${recorte(p.mensagem, 60) || "agora"}`,
      botoes: [
        { texto: "Ok", estilo: "fantasma", acao: "ok-terminei" },
        { texto: "Responder", estilo: "primario", acao: "abrir-responder" },
      ],
    });
    marcarPendente("verde");
    humor("deboche", 3500);
    if (introFeita) abrir(SECAO_CLAUDE);
    // Se em 5 s você não passar o mouse nem clicar em Responder, libera o Claude pra encerrar.
    setTimeout(() => {
      if (pedido !== p || p.segurando || p.respondendo) return;
      resolver({ acao: "ok" });
      marcarPendente("verde"); // o aviso continua lá até você ver
    }, 5000);
  } else if (p.tipo === "permissao") {
    humorFixo = "seria";
    humor("seria");
    marcarPendente("laranja");
    if (introFeita) expandir("aprovacao", htmlAprovacao(p));
  } else if (p.tipo === "pergunta") {
    p.escolhas = (p.perguntas || []).map(() => new Set());
    marcarPendente("roxo");
    if (introFeita) expandir("expandida", htmlPergunta(p));
  }
}

// Fecha o pedido atual (manda a resposta pro Rust) e passa pro próximo da fila.
function resolver(resposta) {
  if (!pedido) return;
  invoke("claude_responder", { id: pedido.id, resposta }).catch(() => {});
  pedido = null;
  humorFixo = null;
  aplicarHumorBase();
  marcarVisto();
  const proximo = fila.shift();
  if (proximo) setTimeout(() => claudePedido(proximo), 400);
}

// Quando abrir a Eva com um pedido esperando, volta pra ele.
function claudeReabrir() {
  if (!pedido || !introFeita) return false;
  if (pedido.tipo === "permissao") { expandir("aprovacao", htmlAprovacao(pedido)); return true; }
  if (pedido.tipo === "pergunta") { expandir("expandida", htmlPergunta(pedido)); return true; }
  if (pedido.tipo === "terminei" && pedido.respondendo) { expandir("expandida", htmlResponder(pedido)); return true; }
  return false;
}

function segurarTerminei() {
  if (pedido?.tipo !== "terminei" || pedido.segurando) return;
  pedido.segurando = true;
  invoke("claude_responder", { id: pedido.id, resposta: { segurar: true } }).catch(() => {});
}

function claudeAoMouseEntrar() {
  // Passou o mouse em cima vendo o Claude: conta como visto e segura o "terminei" pra dar tempo de responder.
  if (secaoAtual !== SECAO_CLAUDE && !expandido()) return;
  if (pedido?.tipo === "terminei") segurarTerminei();
  if (!pedido) marcarVisto();
}

function claudeAoEsconder() {
  // Escondeu sem responder o "terminei": libera o Claude pra encerrar normal.
  if (pedido?.tipo === "terminei" && !pedido.respondendo) {
    const p = pedido;
    resolver({ acao: "ok" });
    desenharCompacto({ selo: "Concluído", cor: "verde", icone: "ok", titulo: "Terminei a tarefa", sub: `${p.projeto} · ${recorte(p.mensagem, 60)}` });
    marcarPendente("verde"); // continua brilhando até você ver
  }
}

// --- cliques ---
function enviarResposta() {
  const caixa = document.getElementById("cl-texto");
  const texto = caixa?.value.trim();
  if (!texto || pedido?.tipo !== "terminei") return;
  resolver({ acao: "responder", texto });
  recolher();
  humor("deboche", 3000);
  mostrarResultado("Mandei pro Claude", `“${recorte(texto, 50)}”`, "verde", "ok");
}

function responderPergunta() {
  const p = pedido;
  if (p?.tipo !== "pergunta") return;
  const respostas = {};
  (p.perguntas || []).forEach((q, qi) => {
    const outra = clExp.querySelector(`[data-outra="${qi}"]`)?.value.trim();
    const escolhidas = [...p.escolhas[qi]].map((oi) => q.options[oi].label);
    const r = outra || escolhidas.join(", ");
    if (r) respostas[q.question] = r;
  });
  if (!Object.keys(respostas).length) return;
  resolver({ acao: "responder", respostas });
  recolher();
  humor("deboche", 3000);
  mostrarResultado("Respondido", recorte(Object.values(respostas).join(" · "), 60), "verde", "ok");
}

function decidirPermissao(acao) {
  const motivo = document.getElementById("cl-motivo")?.value || "";
  resolver({ acao, motivo });
  recolher();
  if (acao === "negar") {
    humor("seria", 3000);
    mostrarResultado("Negado", motivo ? `Motivo: ${recorte(motivo, 50)}` : "Avisei o Claude", "vermelho", "x");
  } else {
    humor("deboche", 3000);
    mostrarResultado(acao === "sempre" ? "Liberado neste projeto" : "Permitido", "O Claude já seguiu", "verde", "ok");
  }
}

function aoClicarClaude(e) {
  const botao = e.target.closest("[data-acao]");
  if (!botao) return;
  marcarVisto();
  const acao = botao.dataset.acao;
  // Trava rápida logo depois que um pedido aparece: um clique duplo (ou atrasado)
  // não decide sem querer o pedido seguinte da fila.
  if (["permitir", "sempre", "negar", "negar-confirmar", "responder-pergunta"].includes(acao)
      && Date.now() - pedidoMostradoEm < 700) return;
  switch (acao) {
    case "ok-aviso": compactoVazio(); break;
    case "ok-terminei": resolver({ acao: "ok" }); compactoVazio(); break;
    case "abrir-responder":
      if (pedido?.tipo !== "terminei") break;
      segurarTerminei();
      pedido.respondendo = true;
      expandir("expandida", htmlResponder(pedido));
      break;
    case "recolher": recolher(); break;
    case "rapida": { const c = document.getElementById("cl-texto"); if (c) { c.value = botao.textContent; c.focus(); } break; }
    case "enviar": enviarResposta(); break;
    case "permitir": decidirPermissao("permitir"); break;
    case "sempre": decidirPermissao("sempre"); break;
    case "negar":
      clExp.querySelector(".cl-motivo").hidden = false;
      claudeAjustarTamanho();
      document.getElementById("cl-motivo")?.focus();
      break;
    case "negar-confirmar": decidirPermissao("negar"); break;
    case "opcao": {
      const p = pedido;
      if (p?.tipo !== "pergunta") break;
      const qi = +botao.dataset.q, oi = +botao.dataset.o, q = p.perguntas[qi];
      if (!q.multiSelect) p.escolhas[qi].clear();
      p.escolhas[qi].has(oi) && q.multiSelect ? p.escolhas[qi].delete(oi) : p.escolhas[qi].add(oi);
      clExp.querySelectorAll(`[data-q="${qi}"]`).forEach((b) => b.classList.toggle("ativa", p.escolhas[qi].has(+b.dataset.o)));
      break;
    }
    case "responder-pergunta": responderPergunta(); break;
    case "no-claude":
      resolver({ acao: "no-claude" });
      recolher();
      mostrarResultado("Pode responder lá no Claude", "Deixei o pedido aparecer por lá", "amarelo", "relogio");
      break;
  }
}
clBotoes.addEventListener("click", aoClicarClaude);
clExp.addEventListener("click", aoClicarClaude);
clExp.addEventListener("toggle", claudeAjustarTamanho, true); // abriu/fechou "ver comando"
clExp.addEventListener("keydown", (e) => {
  if (e.target.id === "cl-texto" && e.key === "Enter" && !e.shiftKey) { e.preventDefault(); enviarResposta(); }
  if (e.target.id === "cl-motivo" && e.key === "Enter") { e.preventDefault(); decidirPermissao("negar"); }
  if (e.key === "Escape" && pedido?.tipo === "terminei") recolher();
});

listen("claude-aviso", (e) => claudeAviso(e.payload));
listen("claude-pedido", (e) => claudePedido(e.payload));
listen("claude-expirou", (e) => {
  const i = fila.findIndex((p) => p.id === e.payload);
  if (i >= 0) fila.splice(i, 1);
  if (pedido?.id !== e.payload) return;
  // Venceu o tempo: o Claude seguiu pelo fluxo normal.
  const tipo = pedido.tipo;
  pedido = null; humorFixo = null; aplicarHumorBase(); marcarVisto();
  if (expandido()) recolher();
  if (tipo === "terminei") compactoVazio();
  else mostrarResultado("O tempo acabou", "O pedido voltou pro próprio Claude", "amarelo", "relogio");
  const proximo = fila.shift();
  if (proximo) setTimeout(() => claudePedido(proximo), 400);
});

compactoVazio();

// ---------------- Geral: relógio + um chip por seção ----------------
const geRelogio = document.getElementById("ge-relogio");
const geDia = document.getElementById("ge-dia");
const geData = document.getElementById("ge-data");
const geControle = document.getElementById("ge-controle");
const geMusica = document.getElementById("ge-musica");
const geClaude = document.getElementById("ge-claude");
const fmtHora = new Intl.DateTimeFormat("pt-BR", { hour: "2-digit", minute: "2-digit" });
const fmtDia = new Intl.DateTimeFormat("pt-BR", { weekday: "long" });
const fmtData = new Intl.DateTimeFormat("pt-BR", { day: "numeric", month: "long" });

function chip(el, texto, classe) {
  el.querySelector("span").textContent = texto;
  el.className = "ge-chip" + (classe ? " " + classe : "");
}

function atualizarGeral() {
  const agora = new Date();
  geRelogio.textContent = fmtHora.format(agora);
  geDia.textContent = fmtDia.format(agora);
  geData.textContent = fmtData.format(agora);

  // Controle: porcentagem com a cor da bateria
  if (ds4.conectado) {
    const nivel = ds4.carregando || ds4.bateria > 50 ? "ok" : ds4.bateria > BATERIA_FRACA ? "medio" : "fraco";
    chip(geControle, ds4.carregando ? `${ds4.bateria}% ⚡` : `${ds4.bateria}%`, nivel);
  } else {
    chip(geControle, "Sem controle", "apagado");
  }

  // Música: o nome do que está tocando
  if (!musica.ativo) chip(geMusica, "Sem música", "apagado");
  else if (!musica.tocando) chip(geMusica, "Pausado", "apagado");
  else chip(geMusica, musica.titulo || musica.app, "");

  // Claude: a mesma cor e texto do selo da seção dele
  const cor = (clSelo.className.match(/verde|amarelo|laranja|vermelho|roxo/) || [])[0];
  const curto = { "Esperando você": "Esperando", "Concluído": "Terminou", "Sempre permitido": "Permitido" };
  const texto = clSelo.textContent;
  chip(geClaude, cor ? curto[texto] || texto : "Tudo certo", cor || "");
}
setInterval(() => { if (modoAtual !== "oculta") atualizarGeral(); }, 1000);
atualizarGeral();

// Clicar num chip leva pra seção dele.
document.querySelector(".ge-chips").addEventListener("click", (e) => {
  const alvo = e.target.closest("[data-ir]");
  if (!alvo) return;
  const i = secoes.findIndex((s) => s.id === alvo.dataset.ir);
  if (i >= 0) irPara(i);
});

// ---------------- Olhos seguindo o mouse ----------------
let olhosLivres = false; // durante a intro ela olha sozinha

function olhar(x, y) {
  mascote.style.setProperty("--olhar-x", `${x}px`);
  mascote.style.setProperty("--olhar-y", `${y}px`);
}

function seguirCursor(cx, cy) {
  if (!olhosLivres || modoAtual === "oculta") return;
  const r = mascote.getBoundingClientRect();
  const olhoX = r.left + r.width / 2;
  const olhoY = r.top + r.height * 0.55;
  // tanh: perto mexe proporcional, longe "satura" no canto do olho
  olhar(3.2 * Math.tanh((cx - olhoX) / 220), 2 * Math.tanh((cy - olhoY) / 220));
}
// O Rust manda a posição do cursor (em px da página) mesmo fora da janela.
listen("eva-cursor", (e) => seguirCursor(e.payload[0], e.payload[1]));

// ---------------- Abertura ----------------
// Nome "EVA" em pixel art: cada letra é uma grade 5×7 de quadradinhos (desenho próprio, sem fonte externa).
const PIXEL = {
  E: ["11111", "10000", "10000", "11110", "10000", "10000", "11111"],
  V: ["10001", "10001", "10001", "10001", "01010", "01010", "00100"],
  A: ["01110", "10001", "10001", "11111", "10001", "10001", "10001"],
};
function letraPixel(letra, tam = 7) {
  const w = 5 * tam, h = 7 * tam;
  let quadrados = "";
  PIXEL[letra].forEach((linha, y) => [...linha].forEach((b, x) => {
    if (b === "1") quadrados += `<rect x="${x * tam}" y="${y * tam}" width="${tam}" height="${tam}"/>`;
  }));
  return `<svg class="eva-letra" viewBox="0 0 ${w} ${h}" width="${w}" height="${h}" aria-hidden="true">
    <defs><linearGradient id="px-${letra}" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#9a8ef0"/>
    </linearGradient></defs>
    <g fill="url(#px-${letra})">${quadrados}</g></svg>`;
}
document.getElementById("eva-nome-letras").innerHTML = ["E", "V", "A"].map((l) => letraPixel(l)).join("");

// Vinheta de entrada: o "acorde suave" (quatro notas juntas entrando devagar, com eco).
function tocarVinheta() {
  try {
    const ctx = new (window.AudioContext || window.webkitAudioContext)();
    ctx.resume?.();
    const t0 = ctx.currentTime + 0.02;
    const mestre = ctx.createGain();
    mestre.gain.value = 0.2;
    mestre.connect(ctx.destination);
    const eco = ctx.createDelay();
    eco.delayTime.value = 0.15;
    const retorno = ctx.createGain();
    retorno.gain.value = 0.28;
    mestre.connect(eco); eco.connect(retorno); retorno.connect(eco); retorno.connect(ctx.destination);
    [261.63, 329.63, 392, 493.88].forEach((freq) => {
      const o = ctx.createOscillator();
      const g = ctx.createGain();
      o.type = "triangle";
      o.frequency.value = freq;
      g.gain.setValueAtTime(0.0001, t0);
      g.gain.linearRampToValueAtTime(0.35, t0 + 0.25);
      g.gain.exponentialRampToValueAtTime(0.0001, t0 + 1.6);
      o.connect(g); g.connect(mestre);
      o.start(t0); o.stop(t0 + 1.7);
    });
    setTimeout(() => ctx.close(), 3000);
  } catch (e) { /* sem som, segue a entrada */ }
}

// Revirar quase inteiro no sentido horário: começa embaixo à direita, passa por baixo,
// sobe pela esquerda, passa por cima e termina à direita (~270°).
const REVIRAR = Array.from({ length: 17 }, (_, i) => {
  const ang = ((-60 - (270 * i) / 16) * Math.PI) / 180;
  return [3 * Math.cos(ang), -2.5 * Math.sin(ang)];
});

const INTRO = [
  [0, () => definirModo("intro")],                                        // 1. desce do topo
  [1250, () => mascote.classList.add("revirando")],
  ...REVIRAR.map(([x, y], i) => [1400 + i * 75, () => olhar(x, y)]),       // 2. revira os olhos
  [2700, () => {                                                          // 3. nome + acorde
    mascote.classList.remove("revirando");
    olhar(0, 1.2);
    definirModo("nome");
    tocarVinheta();
  }],
  [3500, () => { olhar(0, 1.4); humor("deboche", 650); }],                // 4. deboche
  [4150, () => humor("piscadinha", 550)],                                 // 5. piscadinha
  [4700, () => humor("deboche", 600)],
  [5150, () => {                                                          // 6. abre o painel
    olhosLivres = true;
    introFeita = true;
    aplicarHumorBase();
    abrir(0);
  }],
];

irPara(0);
definirModo("inicio");
requestAnimationFrame(() => requestAnimationFrame(() => {
  INTRO.forEach(([ms, passo]) => setTimeout(passo, ms));
}));
invoke("ds4_status").then((s) => s && aplicarDs4(s));
invoke("musica_status").then((r) => {
  if (!r) return;
  aplicarMusica(r[0]);
  aplicarCapa(r[1]);
});

// ---------------- Testes fora do Tauri (navegador) ----------------
if (!temTauri) {
  eva.addEventListener("mouseenter", () => aoMouse(true));
  eva.addEventListener("mouseleave", () => aoMouse(false));
  document.addEventListener("keydown", (e) => e.key === "Insert" && alternar());
  document.addEventListener("mousemove", (e) => seguirCursor(e.clientX, e.clientY));
  document.documentElement.style.background = "radial-gradient(circle at 50% 120%, #b0177f, #3a0a6b 45%, #0b0620 75%)";
}
// No console: testarEva({ bateria: 15 }), humor("seria"), esconder(), abrir(1)...
window.testarEva = (m) => aplicarDs4({ ...ds4, ...m });
window.testarMusica = (m) => aplicarMusica({ ...musica, posicao: musPos, ...m });
window.testarClaude = (p) => claudePedido({ id: Date.now(), projeto: "My Eva", ...p });
window.testarAviso = () => claudeAviso({ projeto: "My Eva" });
window.humor = humor;
window.abrir = abrir;
window.esconder = esconder;
