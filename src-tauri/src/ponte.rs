//! "Interfone" entre o Claude Code e a Eva.
//!
//! A Eva escuta só em 127.0.0.1 (o próprio PC), numa porta aleatória, e grava
//! a porta + uma senha em %APPDATA%\com.myeva.app\ponte.json (Windows) ou
//! ~/.config/com.myeva.app/ponte.json (Linux). O hook do Claude Code
//! lê esse arquivo e manda o que o Claude Code passou pra ele.
//!
//! Rotas:
//!   /aviso     → só avisa ("tô esperando"); responde na hora.
//!   /terminei  → hook Stop: espera uns segundos; se você responder, o Claude continua com seu texto.
//!   /permissao → hook PermissionRequest: espera você permitir/negar.
//!   /pergunta  → hook PreToolUse(AskUserQuestion): espera você escolher as opções.
//!
//! Se ninguém responder a tempo, devolve vazio e o Claude Code segue o fluxo normal.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

struct Pendente {
    tipo: String,
    entrada: Value,
    canal: mpsc::Sender<Value>,
}

#[derive(Default)]
pub struct Ponte {
    pendentes: Mutex<HashMap<u64, Pendente>>,
    proximo: AtomicU64,
    /// Eva ligada? Desligada, todo recado volta vazio e o Claude segue no terminal.
    ligada: AtomicBool,
}
pub type PonteShared = Arc<Ponte>;

impl Ponte {
    pub fn ligar(&self) {
        self.ligada.store(true, Ordering::SeqCst);
    }

    pub fn ligada(&self) -> bool {
        self.ligada.load(Ordering::SeqCst)
    }

    /// Solta tudo o que estava esperando: o canal fecha e o hook devolve vazio.
    pub fn desligar(&self) {
        let mut mapa = self.pendentes.lock().unwrap();
        self.ligada.store(false, Ordering::SeqCst);
        mapa.clear();
    }

    /// Guarda o pedido; com a Eva desligada recusa (devolve false).
    fn guardar(&self, id: u64, pedido: Pendente) -> bool {
        let mut mapa = self.pendentes.lock().unwrap();
        let ligada = self.ligada();
        if ligada {
            mapa.insert(id, pedido);
        }
        ligada
    }

    /// Tem permissão ou pergunta esperando você? ("Terminei" não conta, se resolve sozinho.)
    pub fn tem_pedido(&self) -> bool {
        self.pendentes.lock().unwrap().values().any(|p| p.tipo != "terminei")
    }
}

pub fn iniciar(app: AppHandle, ponte: PonteShared) {
    let servidor = match tiny_http::Server::http("127.0.0.1:0") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[eva] não consegui abrir o interfone: {e}");
            return;
        }
    };
    let porta = servidor.server_addr().to_ip().map(|a| a.port()).unwrap_or(0);
    let Some(token) = gerar_token() else {
        eprintln!("[eva] sem números aleatórios seguros: o interfone não abre");
        return;
    };
    if let Err(e) = gravar_arquivo(porta, &token) {
        eprintln!("[eva] não consegui gravar ponte.json: {e}");
    }
    eprintln!("[eva] interfone ouvindo em 127.0.0.1:{porta}");

    thread::spawn(move || {
        let ocupadas = Arc::new(AtomicUsize::new(0));
        for req in servidor.incoming_requests() {
            // Confere a senha antes de gastar uma thread: recado sem senha morre aqui mesmo.
            if !autorizado(&req, &token) {
                let _ = req.respond(tiny_http::Response::empty(403));
                continue;
            }
            // Limite de recados ao mesmo tempo (bem mais do que o Claude usa de verdade).
            if ocupadas.fetch_add(1, Ordering::SeqCst) >= MAX_RECADOS {
                ocupadas.fetch_sub(1, Ordering::SeqCst);
                let _ = req.respond(tiny_http::Response::empty(503));
                continue;
            }
            let (app, ponte, contador) = (app.clone(), ponte.clone(), ocupadas.clone());
            // Cada recado em sua thread: uma aprovação esperando não trava os outros.
            let criada = thread::Builder::new().name("eva-recado".into()).spawn(move || {
                let _vaga = Vaga(contador);
                atender(req, &app, &ponte);
            });
            if criada.is_err() {
                // A thread não nasceu (o recado se perde e o Claude segue normal), mas o interfone continua.
                ocupadas.fetch_sub(1, Ordering::SeqCst);
            }
        }
    });
}

const MAX_RECADOS: usize = 16;

/// Devolve a vaga quando o recado termina (mesmo se der pânico no meio).
struct Vaga(Arc<AtomicUsize>);
impl Drop for Vaga {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn autorizado(req: &tiny_http::Request, token: &str) -> bool {
    *req.method() == tiny_http::Method::Post
        && req.headers().iter().any(|h| h.field.equiv("X-Eva-Token") && h.value.as_str() == token)
}

/// Chamado pela interface quando você decide alguma coisa.
pub fn responder(ponte: &Ponte, id: u64, resposta: Value) {
    let segurar = resposta.get("segurar").and_then(Value::as_bool).unwrap_or(false);
    let mut mapa = ponte.pendentes.lock().unwrap();
    if segurar {
        if let Some(p) = mapa.get(&id) {
            let _ = p.canal.send(resposta);
        }
        return;
    }
    if let Some(p) = mapa.remove(&id) {
        let saida = montar_saida(&p.tipo, &p.entrada, &resposta);
        let _ = p.canal.send(saida);
    }
}

fn atender(mut req: tiny_http::Request, app: &AppHandle, ponte: &Ponte) {
    let rota = req.url().to_string();
    let mut corpo = String::new();
    let _ = req.as_reader().take(4_000_000).read_to_string(&mut corpo);
    let entrada: Value = serde_json::from_str(&corpo).unwrap_or(Value::Null);

    let saida = match rota.as_str() {
        "/aviso" => {
            let _ = app.emit("claude-aviso", resumo_aviso(&entrada));
            Value::Null
        }
        "/terminei" | "/permissao" | "/pergunta" => {
            let tipo = rota.trim_start_matches('/').to_string();
            esperar_decisao(app, ponte, &tipo, entrada)
        }
        _ => Value::Null,
    };

    let texto = if saida.is_null() { String::new() } else { saida.to_string() };
    let resposta = tiny_http::Response::from_string(texto).with_header(
        tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json; charset=utf-8"[..]).unwrap(),
    );
    let _ = req.respond(resposta);
}

fn esperar_decisao(app: &AppHandle, ponte: &Ponte, tipo: &str, entrada: Value) -> Value {
    let id = ponte.proximo.fetch_add(1, Ordering::Relaxed) + 1;
    let (tx, rx) = mpsc::channel();
    let pedido = montar_pedido(id, tipo, &entrada);
    if !ponte.guardar(id, Pendente { tipo: tipo.to_string(), entrada, canal: tx }) {
        return Value::Null;
    }
    let _ = app.emit("claude-pedido", pedido);

    // "terminei" só segura o Claude uns segundos, a menos que você comece a responder.
    let mut prazo = Instant::now()
        + if tipo == "terminei" { Duration::from_secs(9) } else { Duration::from_secs(580) };

    let resultado = loop {
        let resta = prazo.saturating_duration_since(Instant::now());
        match rx.recv_timeout(resta) {
            Ok(v) if v.get("segurar").and_then(Value::as_bool) == Some(true) => {
                prazo = Instant::now() + Duration::from_secs(580);
            }
            Ok(v) => break v,
            Err(_) => break Value::Null,
        }
    };
    if ponte.pendentes.lock().unwrap().remove(&id).is_some() {
        // Expirou sem resposta: avisa a interface pra fechar o pedido.
        let _ = app.emit("claude-expirou", id);
    }
    resultado
}

// ---------------------------------------------------------------------------
// O que a interface mostra
// ---------------------------------------------------------------------------

fn projeto(entrada: &Value) -> String {
    let cwd = entrada.get("cwd").and_then(Value::as_str).unwrap_or("");
    cwd.trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("Claude")
        .to_string()
}

fn cortar(texto: &str, max: usize) -> String {
    let t = texto.trim();
    if t.chars().count() <= max {
        return t.to_string();
    }
    let mut s: String = t.chars().take(max).collect();
    s.push('…');
    s
}

fn resumo_aviso(entrada: &Value) -> Value {
    json!({
        "tipo": "espera",
        "projeto": projeto(entrada),
        "mensagem": entrada.get("message").and_then(Value::as_str).unwrap_or(""),
    })
}

fn montar_pedido(id: u64, tipo: &str, entrada: &Value) -> Value {
    let proj = projeto(entrada);
    match tipo {
        "terminei" => json!({
            "id": id, "tipo": "terminei", "projeto": proj,
            "mensagem": cortar(entrada.get("last_assistant_message").and_then(Value::as_str).unwrap_or(""), 700),
        }),
        "pergunta" => json!({
            "id": id, "tipo": "pergunta", "projeto": proj,
            "perguntas": entrada.pointer("/tool_input/questions").cloned().unwrap_or(json!([])),
        }),
        _ => {
            let ferramenta = entrada.get("tool_name").and_then(Value::as_str).unwrap_or("?").to_string();
            let ti = entrada.get("tool_input").cloned().unwrap_or(Value::Null);
            let comando = ["command", "file_path", "notebook_path", "url", "pattern", "path"]
                .iter()
                .find_map(|k| ti.get(*k).and_then(Value::as_str))
                .unwrap_or("")
                .to_string();
            let (nivel, explicacao, efeitos) = avaliar_risco(&ferramenta, &comando);
            // O resumo é montado pela Eva a partir do pedido real; o texto do próprio Claude
            // aparece só como comentário dele, nunca no lugar do que vai acontecer.
            let resumo = resumo_padrao(&ferramenta, &comando);
            let nota = ti.get("description").and_then(Value::as_str).unwrap_or("");
            let (detalhes, detalhes_cortados) = detalhes_da_ferramenta(&ferramenta, &ti);
            // Pedido grande demais pra mostrar inteiro: nada de regra permanente.
            let regras = if detalhes_cortados { None } else { regras_sempre(entrada) };
            let porque = entrada
                .get("transcript_path")
                .and_then(Value::as_str)
                .map(ultima_fala_do_claude)
                .unwrap_or_default();
            json!({
                "id": id, "tipo": "permissao", "projeto": proj,
                "cwd": entrada.get("cwd").and_then(Value::as_str).unwrap_or(""),
                "ferramenta": ferramenta, "comando": comando,
                "resumo": resumo, "nota": cortar(nota, 240), "porque": cortar(&porque, 420),
                "risco": nivel, "riscoTexto": explicacao, "efeitos": efeitos,
                "detalhes": detalhes, "detalhesCortados": detalhes_cortados,
                "podeSempre": nivel != "alto" && regras.is_some(),
                "regra": regras.as_ref().map(descrever_regras).unwrap_or_default(),
            })
        }
    }
}

fn resumo_padrao(ferramenta: &str, alvo: &str) -> String {
    let nome = alvo.rsplit(['\\', '/']).next().unwrap_or(alvo);
    match ferramenta {
        "Edit" | "MultiEdit" => format!("Editar o arquivo {nome}"),
        "Write" => format!("Criar ou sobrescrever o arquivo {nome}"),
        "NotebookEdit" => format!("Editar o notebook {nome}"),
        "WebFetch" => format!("Abrir a página {alvo}"),
        "WebSearch" => "Pesquisar na internet".to_string(),
        "Bash" | "PowerShell" => "Rodar um comando no terminal".to_string(),
        f if f.starts_with("mcp__") => format!("Usar a ferramenta {}", f.rsplit("__").next().unwrap_or(f)),
        f => format!("Usar a ferramenta {f}"),
    }
}

/// Classifica o pedido em baixo / médio / alto, com uma frase e o que muda no PC.
fn avaliar_risco(ferramenta: &str, alvo: &str) -> (&'static str, String, Vec<String>) {
    let c = format!(" {} ", alvo.to_lowercase());
    let tem = |pedacos: &[&str]| pedacos.iter().any(|p| c.contains(p));

    match ferramenta {
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => {
            return (
                "medio",
                "Muda um arquivo do projeto. Dá pra desfazer pelo histórico.".into(),
                vec![format!("Altera {}", alvo.rsplit(['\\', '/']).next().unwrap_or(alvo))],
            )
        }
        "WebFetch" | "WebSearch" => {
            return ("baixo", "Só lê da internet. Não muda nada no seu PC.".into(), vec!["Acessa a internet".into()])
        }
        "Bash" | "PowerShell" => {}
        _ => {
            return (
                "medio",
                "Ferramenta externa: confira os detalhes abaixo antes de permitir.".into(),
                vec!["Pode enviar, alterar ou apagar dados fora do projeto".into()],
            )
        }
    }

    if tem(&[
        " rm ", " rm -", "remove-item", " del ", " rmdir", " rd ", " erase ", "format ", "git push --force",
        "git push -f", "git reset --hard", "git clean", "drop table", "drop database", "reg delete",
        "shutdown", "restart-computer", "clear-content", "--force-with-lease", " ri ", "rmtree", "unlink",
        "delete", "-recurse", "remove-", "stop-computer", "diskpart", "bcdedit", "takeown", "icacls",
        "-encodedcommand", " -enc ", "invoke-expression", " iex ", "iex(", "| sh", "| bash", "|sh", "|bash",
        "| iex", "|iex", "set-executionpolicy", "certutil", "bitsadmin", "schtasks", "new-service",
        // Linux
        "sudo ", "pkexec", " su ", " dd ", "mkfs", "wipefs", "fdisk", "parted", "shred", "truncate ",
        "chmod -r", "chown -r", "> /dev/sd", "> /dev/nvme", "> /etc/", "tee /etc/", "systemctl", "crontab", "reboot",
        "poweroff", " halt", "apt remove", "apt purge", "apt-get remove", "apt-get purge", "snap remove",
        "flatpak uninstall", "find -delete", " -delete", "xargs rm",
    ]) {
        return (
            "alto",
            "Apaga ou sobrescreve coisas, ou mexe no sistema. Pode não ter volta.".into(),
            vec!["Pode apagar arquivos ou desfazer histórico".into(), "Confira o comando exato antes".into()],
        );
    }
    if tem(&[
        "install", " add ", " npm i ", "pip ", "winget", "choco ", "curl ", "wget ", "invoke-webrequest",
        "iwr ", "git push", "git commit", " mv ", "move-item", "rename", "set-content", "out-file",
        " > ", " >> ", "stop-process", "taskkill", "npx ", "python -c", "python3 -c", "node -e", "-command",
        "start-process", "cmd /c", "cmd.exe", "powershell", "pwsh", "bash -c", "sh -c", "&&", "||", ";", "|",
        "`", "$(", "eval ",
        // Linux
        "apt ", "apt-get ", "snap ", "flatpak ", "dnf ", "pacman ", "chmod ", "chown ", " cp ", " ln ",
        "kill ", "pkill", "killall", "nohup", "source ", "exec ",
    ]) {
        return (
            "medio",
            "Instala, baixa da internet ou muda arquivos. Normalmente dá pra desfazer.".into(),
            vec!["Pode baixar coisas ou alterar arquivos do projeto".into()],
        );
    }
    (
        "baixo",
        "Não reconheci nada perigoso, mas isso é só uma estimativa: confira o comando.".into(),
        vec!["Roda no terminal do projeto".into()],
    )
}

/// Pega a última coisa que o Claude escreveu antes do pedido (o "por quê").
fn ultima_fala_do_claude(caminho: &str) -> String {
    // Lê só o finalzinho do histórico (pode ter centenas de MB).
    const FIM: u64 = 512 * 1024;
    let Ok(mut arquivo) = std::fs::File::open(caminho) else { return String::new() };
    let tamanho = arquivo.metadata().map(|m| m.len()).unwrap_or(0);
    let inicio = tamanho.saturating_sub(FIM);
    if arquivo.seek(SeekFrom::Start(inicio)).is_err() {
        return String::new();
    }
    let mut bruto = Vec::new();
    if arquivo.take(FIM).read_to_end(&mut bruto).is_err() {
        return String::new();
    }
    let texto = String::from_utf8_lossy(&bruto);
    // Se começou no meio, a primeira linha está pela metade: pula.
    let texto = if inicio > 0 { texto.split_once('\n').map(|(_, r)| r).unwrap_or("") } else { &texto };
    for linha in texto.lines().rev().take(400) {
        let Ok(v) = serde_json::from_str::<Value>(linha) else { continue };
        if v.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        if let Some(partes) = v.pointer("/message/content").and_then(Value::as_array) {
            let falas: Vec<&str> = partes
                .iter()
                .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect();
            if !falas.is_empty() {
                return falas.join(" ");
            }
        }
    }
    String::new()
}

// ---------------------------------------------------------------------------
// O que volta pro Claude Code
// ---------------------------------------------------------------------------

fn montar_saida(tipo: &str, entrada: &Value, r: &Value) -> Value {
    let acao = r.get("acao").and_then(Value::as_str).unwrap_or("");
    match (tipo, acao) {
        // Hook Stop: seu texto vira o motivo pra eu continuar.
        ("terminei", "responder") => {
            let texto = r.get("texto").and_then(Value::as_str).unwrap_or("").trim();
            if texto.is_empty() {
                return Value::Null;
            }
            json!({ "decision": "block", "reason": format!("Mensagem do usuário, enviada pela Eva: {texto}") })
        }
        ("permissao", "permitir") => permissao(json!({ "behavior": "allow" })),
        // Só grava uma regra se ela for exata (a mesma que apareceu na tela).
        // Sem regra exata, vale como "permitir uma vez".
        ("permissao", "sempre") => match regras_sempre(entrada) {
            Some(regras) => permissao(json!({ "behavior": "allow", "updatedPermissions": regras })),
            None => permissao(json!({ "behavior": "allow" })),
        },
        ("permissao", "negar") => {
            let motivo = r.get("motivo").and_then(Value::as_str).unwrap_or("").trim();
            let mensagem = if motivo.is_empty() {
                "O usuário negou pela Eva.".to_string()
            } else {
                format!("O usuário negou pela Eva. Motivo: {motivo}")
            };
            permissao(json!({ "behavior": "deny", "message": mensagem }))
        }
        // Hook PreToolUse do AskUserQuestion: devolve as perguntas com as respostas.
        ("pergunta", "responder") => {
            let Some(perguntas) = entrada.pointer("/tool_input/questions").cloned() else { return Value::Null };
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "allow",
                    "updatedInput": { "questions": perguntas, "answers": r.get("respostas").cloned().unwrap_or(json!({})) }
                }
            })
        }
        // "Responder no Claude", dispensar, etc.: deixa o fluxo normal seguir.
        _ => Value::Null,
    }
}

fn permissao(decisao: Value) -> Value {
    json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decisao } })
}

// ---------------------------------------------------------------------------

/// 256 bits do gerador criptográfico do sistema. Sem ele, não há ponte (nunca senha fraca).
fn gerar_token() -> Option<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).ok()?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

pub fn arquivo_ponte() -> PathBuf {
    pasta_config().join("com.myeva.app").join("ponte.json")
}

/// %APPDATA% no Windows; $XDG_CONFIG_HOME (ou ~/.config) no Linux.
fn pasta_config() -> PathBuf {
    let var = |nome: &str| std::env::var(nome).ok().filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        var("APPDATA")
    } else {
        var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|h| h.join(".config")))
    }
    .unwrap_or_else(std::env::temp_dir)
}

fn gravar_arquivo(porta: u16, token: &str) -> std::io::Result<()> {
    let caminho = arquivo_ponte();
    if let Some(pasta) = caminho.parent() {
        std::fs::create_dir_all(pasta)?;
        // No Linux, outros usuários do PC não podem nem listar a pasta.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(pasta, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let conteudo = json!({ "porta": porta, "token": token }).to_string();
    #[cfg(unix)]
    {
        // Cria (ou recria) o arquivo já com permissão só pro seu usuário (0600).
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let _ = std::fs::remove_file(&caminho);
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&caminho)?;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        return f.write_all(conteudo.as_bytes());
    }
    #[cfg(not(unix))]
    std::fs::write(caminho, conteudo)
}

// ---------------------------------------------------------------------------
// "Sempre permitir": só regras exatas
// ---------------------------------------------------------------------------

/// A regra que o "Sempre permitir" grava, ou None se não der pra ser exata.
/// - Terminal (Bash/PowerShell): sempre o comando inteiro, igualzinho; as sugestões do
///   Claude Code são ignoradas (elas costumam ser prefixos com curinga, tipo `git:*`).
/// - Outras ferramentas: só sugestões sem nenhum curinga, da mesma ferramenta do pedido,
///   gravadas só pra você (localSettings) ou só nesta sessão.
/// Nunca libera ferramenta inteira, curinga, troca de modo, pastas ou regra compartilhada.
fn regras_sempre(entrada: &Value) -> Option<Value> {
    let ferramenta = entrada.get("tool_name").and_then(Value::as_str).unwrap_or("").trim();
    if ferramenta.is_empty() {
        return None;
    }
    let regras = if matches!(ferramenta, "Bash" | "PowerShell") {
        let cmd = entrada.pointer("/tool_input/command").and_then(Value::as_str)?.trim();
        if cmd.is_empty() {
            return None;
        }
        json!([{
            "type": "addRules",
            "rules": [{ "toolName": ferramenta, "ruleContent": cmd }],
            "behavior": "allow",
            "destination": "localSettings",
        }])
    } else {
        let lista = entrada.get("permission_suggestions").and_then(Value::as_array)?;
        if lista.is_empty() || !lista.iter().all(|s| sugestao_exata(s, ferramenta)) {
            return None;
        }
        Value::Array(lista.clone())
    };
    regras_cabem_na_tela(&regras).then_some(regras)
}

const MAX_REGRA: usize = 300;

fn sugestao_exata(s: &Value, ferramenta: &str) -> bool {
    let campo = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or("");
    let destino_ok = matches!(campo("destination"), "localSettings" | "session");
    let regras_ok = s.get("rules").and_then(Value::as_array).is_some_and(|rs| {
        !rs.is_empty()
            && rs.iter().all(|r| {
                let nome = r.get("toolName").and_then(Value::as_str).unwrap_or("").trim();
                let alvo = r.get("ruleContent").and_then(Value::as_str).unwrap_or("").trim();
                nome == ferramenta && !alvo.is_empty() && !alvo.contains('*')
            })
    });
    campo("type") == "addRules" && campo("behavior") == "allow" && destino_ok && regras_ok
}

/// Regra longa demais pra conferir com calma na tela: não oferece "Sempre".
fn regras_cabem_na_tela(regras: &Value) -> bool {
    regras.as_array().into_iter().flatten().all(|s| {
        s.get("rules").and_then(Value::as_array).into_iter().flatten().all(|r| {
            r.get("ruleContent").and_then(Value::as_str).unwrap_or("").chars().count() <= MAX_REGRA
        })
    })
}

/// Texto da regra pra mostrar antes de você confirmar, sempre inteiro (sem cortar),
/// ex.: "Bash(npm test) · só pra você, neste projeto".
fn descrever_regras(regras: &Value) -> String {
    let mut partes = Vec::new();
    for s in regras.as_array().into_iter().flatten() {
        let destino = match s.get("destination").and_then(Value::as_str).unwrap_or("") {
            "session" => "só nesta sessão",
            _ => "só pra você, neste projeto",
        };
        for r in s.get("rules").and_then(Value::as_array).into_iter().flatten() {
            let nome = r.get("toolName").and_then(Value::as_str).unwrap_or("?");
            let alvo = r.get("ruleContent").and_then(Value::as_str).unwrap_or("");
            partes.push(format!("{nome}({alvo}) · {destino}"));
        }
    }
    partes.join("\n")
}

/// Todos os argumentos do pedido (menos a descrição do próprio Claude), pra você ver
/// exatamente o que vai ser enviado ou alterado. Terminal já aparece no comando.
const MAX_DETALHES: usize = 4000;

fn detalhes_da_ferramenta(ferramenta: &str, ti: &Value) -> (String, bool) {
    let Some(obj) = ti.as_object() else { return (String::new(), false) };
    let mut resto = obj.clone();
    resto.remove("description");
    if matches!(ferramenta, "Bash" | "PowerShell") {
        resto.remove("command");
        resto.remove("timeout");
        resto.remove("run_in_background");
    }
    if resto.is_empty() {
        return (String::new(), false);
    }
    let texto = serde_json::to_string_pretty(&Value::Object(resto)).unwrap_or_default();
    let total = texto.chars().count();
    if total <= MAX_DETALHES {
        (texto, false)
    } else {
        let mut t: String = texto.chars().take(MAX_DETALHES).collect();
        t.push_str(&format!("\n… (mais {} caracteres: confira no próprio Claude)", total - MAX_DETALHES));
        (t, true)
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn pedido(ferramenta: &str, entrada_ferramenta: Value, sugestoes: Value) -> Value {
        json!({ "tool_name": ferramenta, "tool_input": entrada_ferramenta, "permission_suggestions": sugestoes, "cwd": "D:/x" })
    }

    #[test]
    fn desligar_solta_os_pedidos_e_recusa_novos() {
        let ponte = Ponte::default();
        let novo = |tipo: &str| {
            let (tx, rx) = mpsc::channel();
            (Pendente { tipo: tipo.into(), entrada: Value::Null, canal: tx }, rx)
        };
        let (p, _rx) = novo("permissao");
        assert!(!ponte.guardar(1, p), "desligada não aceita pedido");

        ponte.ligar();
        let (p, _rx) = novo("terminei");
        assert!(ponte.guardar(2, p));
        assert!(!ponte.tem_pedido(), "terminei não conta como pedido");
        let (p, rx) = novo("permissao");
        assert!(ponte.guardar(3, p));
        assert!(ponte.tem_pedido());

        ponte.desligar();
        assert!(!ponte.tem_pedido());
        assert!(matches!(rx.recv(), Err(mpsc::RecvError)), "quem esperava é solto na hora");
        let (p, _rx) = novo("pergunta");
        assert!(!ponte.guardar(4, p));
    }

    #[test]
    fn resumo_ignora_descricao_do_claude() {
        let e = pedido("Bash", json!({ "command": "rm -rf C:/dados", "description": "Listar arquivos" }), json!([]));
        let p = montar_pedido(1, "permissao", &e);
        assert_eq!(p["resumo"], "Rodar um comando no terminal");
        assert_eq!(p["nota"], "Listar arquivos");
        assert_eq!(p["comando"], "rm -rf C:/dados");
    }

    #[test]
    fn arquivo_sem_sugestao_nao_vira_regra_da_ferramenta_inteira() {
        let e = pedido("Write", json!({ "file_path": "D:/x/a.txt" }), json!([]));
        assert!(regras_sempre(&e).is_none());
        assert_eq!(montar_pedido(1, "permissao", &e)["podeSempre"], false);
        let saida = montar_saida("permissao", &e, &json!({ "acao": "sempre" }));
        assert!(saida.pointer("/hookSpecificOutput/decision/updatedPermissions").is_none());
        assert_eq!(saida.pointer("/hookSpecificOutput/decision/behavior").unwrap(), "allow");
    }

    #[test]
    fn comando_sem_sugestao_vira_regra_exata() {
        let e = pedido("Bash", json!({ "command": "npm test" }), Value::Null);
        let r = regras_sempre(&e).unwrap();
        assert_eq!(r[0]["rules"][0]["ruleContent"], "npm test");
        assert_eq!(descrever_regras(&r), "Bash(npm test) · só pra você, neste projeto");
    }

    #[test]
    fn sugestoes_amplas_sao_recusadas() {
        let modo = json!([{ "type": "setMode", "mode": "acceptEdits", "destination": "session" }]);
        let sem_alvo = json!([{ "type": "addRules", "rules": [{ "toolName": "Edit" }], "behavior": "allow", "destination": "localSettings" }]);
        let curinga = json!([{ "type": "addRules", "rules": [{ "toolName": "Edit", "ruleContent": "src/**" }], "behavior": "allow", "destination": "localSettings" }]);
        let pastas = json!([{ "type": "addDirectories", "directories": ["C:/"], "destination": "session" }]);
        for s in [modo, sem_alvo, curinga, pastas] {
            assert!(regras_sempre(&pedido("Edit", json!({ "file_path": "D:/x/a.txt" }), s)).is_none());
        }
        let boa = json!([{ "type": "addRules", "rules": [{ "toolName": "Edit", "ruleContent": "/x/a.txt" }], "behavior": "allow", "destination": "session" }]);
        let r = regras_sempre(&pedido("Edit", json!({ "file_path": "D:/x/a.txt" }), boa)).unwrap();
        assert_eq!(descrever_regras(&r), "Edit(/x/a.txt) · só nesta sessão");
    }

    fn regra(tool: &str, alvo: &str, dest: &str) -> Value {
        json!([{ "type": "addRules", "rules": [{ "toolName": tool, "ruleContent": alvo }], "behavior": "allow", "destination": dest }])
    }

    #[test]
    fn terminal_ignora_sugestao_com_curinga_e_grava_comando_exato() {
        for s in [regra("Bash", "git:*", "localSettings"), regra("Bash", "git * --force", "localSettings"), regra("Edit", "//**", "localSettings")] {
            let r = regras_sempre(&pedido("Bash", json!({ "command": "git status" }), s)).unwrap();
            assert_eq!(descrever_regras(&r), "Bash(git status) · só pra você, neste projeto");
        }
    }

    #[test]
    fn outras_ferramentas_so_aceitam_regra_exata_da_mesma_ferramenta() {
        let recusadas = [
            regra("WebFetch", "domain:*", "localSettings"),
            regra("Read", "~/**", "localSettings"),
            regra("Edit", "//**", "localSettings"),
            regra("WebFetch", "domain:exemplo.invalid", "projectSettings"),
            regra("Bash", "rm:*", "localSettings"),
        ];
        for s in recusadas {
            assert!(regras_sempre(&pedido("WebFetch", json!({ "url": "https://exemplo.invalid/a" }), s)).is_none());
        }
        let boa = regra("WebFetch", "domain:exemplo.invalid", "localSettings");
        assert!(regras_sempre(&pedido("WebFetch", json!({ "url": "https://exemplo.invalid/a" }), boa)).is_some());
    }

    #[test]
    fn regra_longa_nao_vira_sempre_e_previa_nao_corta() {
        let longo = format!("npm run build -- {} && echo fim", "a/".repeat(200));
        let e = pedido("Bash", json!({ "command": longo }), json!([]));
        assert!(regras_sempre(&e).is_none());
        assert_eq!(montar_pedido(1, "permissao", &e)["podeSempre"], false);
        let medio = format!("echo {}fim", "b".repeat(250));
        let r = regras_sempre(&pedido("Bash", json!({ "command": medio.clone() }), json!([]))).unwrap();
        assert!(descrever_regras(&r).contains(&format!("{medio})")));
    }

    #[test]
    fn mcp_mostra_todos_os_argumentos() {
        let e = pedido("mcp__correio__enviar", json!({ "para": "alguem@exemplo.invalid", "assunto": "Oi", "description": "Teste" }), json!([]));
        let p = montar_pedido(1, "permissao", &e);
        let d = p["detalhes"].as_str().unwrap();
        assert!(d.contains("alguem@exemplo.invalid") && d.contains("assunto") && !d.contains("Teste"));
        assert_eq!(p["comando"], "");
    }

    #[test]
    fn risco_desconhecido_nao_promete_seguranca() {
        assert_eq!(avaliar_risco("Bash", "python -c \"import shutil; shutil.rmtree('x')\"").0, "alto");
        assert_eq!(avaliar_risco("PowerShell", "ri -r pasta").0, "alto");
        assert_eq!(avaliar_risco("Bash", "echo a && echo b").0, "medio");
        assert!(!avaliar_risco("Bash", "ls").1.contains("Não apaga nada"));
    }

    #[test]
    fn historico_le_so_o_final() {
        let dir = std::env::temp_dir().join(format!("eva-teste-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let arq = dir.join("t.jsonl");
        let mut texto = String::new();
        for _ in 0..20000 {
            texto.push_str("{\"type\":\"user\",\"message\":{\"content\":\"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\"}}\n");
        }
        texto.push_str("{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Vou rodar os testes.\"}]}}\n");
        std::fs::write(&arq, texto).unwrap();
        assert_eq!(ultima_fala_do_claude(arq.to_str().unwrap()), "Vou rodar os testes.");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
