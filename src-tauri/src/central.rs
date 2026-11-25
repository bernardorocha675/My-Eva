//! Central: a janela que abre com o executável. Ela liga e desliga a Eva.
//!
//! - Ligar: cria a janela da Eva ("main") e fecha a Central.
//! - Desligar (ou "Sair da Eva" na bandeja): a Eva sobe, some e a Central volta.
//! - Fechar a Central com a Eva ligada fecha só ela; desligada, era a última janela e o app sai.
//!
//! As janelas são destruídas de verdade ao fechar, pra não ficarem ocupando memória.
//! Criar janela dentro do loop principal trava no Windows: por isso os comandos são `async`
//! e a bandeja usa uma thread.

use crate::{ponte, AreaShared};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;
use tauri::menu::CheckMenuItem;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow, WebviewWindowBuilder, Wry};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Shortcut};

pub const BANDEJA: &str = "eva";

#[derive(Default)]
pub struct Controle {
    /// A Central deve abrir já perguntando se desliga com pedido esperando.
    perguntar: AtomicBool,
    /// Música, controle e ponte só começam na primeira vez que a Eva liga.
    servicos: AtomicBool,
    /// Já tem alguém ligando ou desligando.
    ocupado: AtomicBool,
}

/// Item "Iniciar com o sistema" da bandeja, pra ficar igual ao da Central.
pub struct MenuInicio(pub CheckMenuItem<Wry>);

#[derive(Serialize)]
pub struct EstadoCentral {
    ligada: bool,
    versao: String,
    inicia: bool,
    perguntar: bool,
}

fn erro(e: impl std::fmt::Display) -> String {
    eprintln!("[eva] central: {e}");
    e.to_string()
}

pub fn ligada(app: &AppHandle) -> bool {
    app.get_webview_window("main").is_some()
}

fn criar(app: &AppHandle, nome: &str) -> tauri::Result<WebviewWindow> {
    let cfg = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == nome)
        .cloned()
        .ok_or(tauri::Error::WindowNotFound)?;
    WebviewWindowBuilder::from_config(app, &cfg)?.build()
}

pub fn abrir(app: &AppHandle) -> tauri::Result<()> {
    match app.get_webview_window("central") {
        Some(j) => {
            let _ = j.unminimize();
            let _ = j.show();
            let _ = j.set_focus();
            let _ = j.emit("central-mudou", ());
            Ok(())
        }
        None => {
            // Sem recorte de cliques: ele podia ficar no tamanho errado quando a escala da tela
            // muda logo depois do login, e aí nenhum botão funcionava.
            let janela = criar(app, "central")?;
            #[cfg(target_os = "linux")]
            crate::linux::vigiar_escala_central(&janela);
            #[cfg(not(target_os = "linux"))]
            let _ = janela;
            Ok(())
        }
    }
}

/// Pra chamar de dentro do loop principal (bandeja, segunda abertura do programa).
pub fn abrir_em_thread(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || {
        let _ = abrir(&app).map_err(erro);
    });
}

/// Garante um liga/desliga por vez.
struct Vez<'a>(&'a AtomicBool);
impl<'a> Vez<'a> {
    fn pegar(trava: &'a AtomicBool) -> Option<Self> {
        (!trava.swap(true, Ordering::SeqCst)).then_some(Self(trava))
    }
}
impl Drop for Vez<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn ligar(app: &AppHandle) -> Result<(), String> {
    let controle = app.state::<Controle>();
    let Some(_vez) = Vez::pegar(&controle.ocupado) else { return Err("ocupada".into()) };
    if app.state::<crate::atualizacao::Atualizacao>().trabalhando() {
        return Err("atualizando".into());
    }
    if !ligada(app) {
        *app.state::<AreaShared>().lock().unwrap() = Default::default();
        let janela = criar(app, "main").map_err(erro)?;
        app.state::<ponte::PonteShared>().ligar();
        crate::preparar_eva(app, &janela);
        if !controle.servicos.swap(true, Ordering::SeqCst) {
            crate::iniciar_servicos(app);
        }
        // Tecla INS: só enquanto a Eva está ligada.
        if let Err(e) = app.global_shortcut().register(Shortcut::new(None, Code::Insert)) {
            eprintln!("[eva] não consegui registrar a tecla INS: {e}");
        }
        mostrar_bandeja(app, true);
    }
    if let Some(c) = app.get_webview_window("central") {
        let _ = c.destroy();
    }
    Ok(())
}

/// Devolve false quando tem pedido do Claude esperando e ainda não foi confirmado.
fn desligar(app: &AppHandle, confirmado: bool) -> Result<bool, String> {
    let controle = app.state::<Controle>();
    let Some(_vez) = Vez::pegar(&controle.ocupado) else { return Ok(true) };
    let Some(janela) = app.get_webview_window("main") else { return Ok(true) };
    let ponte = app.state::<ponte::PonteShared>();
    if !confirmado && ponte.tem_pedido() {
        controle.perguntar.store(true, Ordering::SeqCst);
        abrir(app).map_err(erro)?;
        return Ok(false);
    }
    // Os pedidos que sobraram voltam pro terminal na hora.
    ponte.desligar();
    let _ = janela.emit("eva-despedir", ());
    // A Central vem antes de a Eva sumir, senão o app fica sem janela e sai.
    if let Err(e) = abrir(app) {
        ponte.ligar();
        return Err(erro(e));
    }
    thread::sleep(Duration::from_millis(900));
    let _ = janela.destroy();
    Ok(true)
}

pub fn desligar_em_thread(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || {
        let _ = desligar(&app, false);
    });
}

/// A janela da Eva acabou (por qualquer motivo): arruma o resto.
pub fn eva_fechou(app: &AppHandle) {
    app.state::<ponte::PonteShared>().desligar();
    let _ = app.global_shortcut().unregister(Shortcut::new(None, Code::Insert));
    mostrar_bandeja(app, false);
    let _ = app.emit("central-mudou", ());
}

fn mostrar_bandeja(app: &AppHandle, visivel: bool) {
    if let Some(b) = app.tray_by_id(BANDEJA) {
        let _ = b.set_visible(visivel);
    }
}

pub fn definir_inicio(app: &AppHandle, ligar: bool) -> bool {
    let gerente = app.autolaunch();
    let _ = if ligar { gerente.enable() } else { gerente.disable() };
    let agora = gerente.is_enabled().unwrap_or(false);
    if let Some(m) = app.try_state::<MenuInicio>() {
        let _ = m.0.set_checked(agora);
    }
    let _ = app.emit("central-mudou", ());
    agora
}

#[tauri::command]
pub fn central_estado(app: AppHandle) -> EstadoCentral {
    EstadoCentral {
        ligada: ligada(&app) && app.state::<ponte::PonteShared>().ligada(),
        versao: app.package_info().version.to_string(),
        inicia: app.autolaunch().is_enabled().unwrap_or(false),
        perguntar: app.state::<Controle>().perguntar.swap(false, Ordering::SeqCst),
    }
}

#[tauri::command]
pub async fn central_ligar(app: AppHandle) -> Result<(), String> {
    ligar(&app)
}

#[tauri::command]
pub async fn central_desligar(app: AppHandle, confirmado: bool) -> Result<bool, String> {
    desligar(&app, confirmado)
}

#[tauri::command]
pub fn central_ver_pedido(app: AppHandle) {
    let _ = app.emit_to("main", "eva-abrir", ());
    central_fechar(app);
}

#[tauri::command]
pub fn central_fechar(app: AppHandle) {
    if let Some(c) = app.get_webview_window("central") {
        let _ = c.close();
    }
}

#[tauri::command]
pub fn central_inicio(app: AppHandle, ligar: bool) -> bool {
    definir_inicio(&app, ligar)
}
