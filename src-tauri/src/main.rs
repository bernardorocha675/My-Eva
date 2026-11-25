// Sem janela de console (nem na versão de testes). O diário vai pro log quando redirecionado.
#![windows_subsystem = "windows"]

mod atualizacao;
mod central;
// Cada sistema tem a sua versão da bateria do controle e da música;
// as duas expõem a mesma interface pro resto do app.
#[cfg_attr(windows, path = "ds4_windows.rs")]
#[cfg_attr(target_os = "linux", path = "ds4_linux.rs")]
mod ds4;
#[cfg_attr(windows, path = "musica_windows.rs")]
#[cfg_attr(target_os = "linux", path = "musica_linux.rs")]
mod musica;
mod ponte;

use serde::Deserialize;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri_plugin_autostart::ManagerExt;
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow};
use tauri_plugin_global_shortcut::ShortcutState;

/// Retângulo (em px da página) onde a Eva está visível agora.
/// Fora dele, os cliques passam direto pra o que está atrás.
#[derive(Clone, Copy, Default, Deserialize)]
struct Area {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}
type AreaShared = Arc<Mutex<Area>>;

#[tauri::command]
fn ds4_status(estado: tauri::State<ds4::Shared>) -> ds4::Ds4Status {
    estado.lock().unwrap().clone()
}

#[tauri::command]
fn musica_status(estado: tauri::State<musica::Shared>) -> (musica::MusicaStatus, Option<String>) {
    let e = estado.lock().unwrap();
    (e.status.clone(), e.capa.clone())
}

#[tauri::command]
fn musica_acao(acao: String, app: AppHandle) {
    let Some(canal) = app.try_state::<musica::Canal>() else { return };
    let a = match acao.as_str() {
        "tocar_pausar" => musica::Acao::TocarPausar,
        "proxima" => musica::Acao::Proxima,
        "anterior" => musica::Acao::Anterior,
        _ => return,
    };
    let _ = canal.lock().unwrap().send(a);
}

#[tauri::command]
fn claude_responder(id: u64, resposta: serde_json::Value, ponte: tauri::State<ponte::PonteShared>) {
    ponte::responder(&ponte, id, resposta);
}

#[tauri::command]
fn definir_area(area: Area, estado: tauri::State<AreaShared>, janela: WebviewWindow) {
    *estado.lock().unwrap() = area;
    #[cfg(target_os = "linux")]
    linux::aplicar_regiao(&janela, area);
    #[cfg(not(target_os = "linux"))]
    let _ = janela;
}

/// Coisas que só existem no Linux.
#[cfg(target_os = "linux")]
mod linux {
    use super::{Area, AreaShared};
    use gtk::prelude::{WidgetExt, WidgetExtManual};
    use gtk::prelude::{Cast, MonitorExt};
    use std::sync::Mutex;
    use std::time::{Duration, Instant, SystemTime};
    use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

    /// Onde o mouse está, pelos eventos do GTK (None = fora da Eva).
    static PONTEIRO: Mutex<Option<(f64, f64)>> = Mutex::new(None);
    /// Último aviso mandado pra interface, pra só avisar quando muda.
    static DENTRO: Mutex<bool> = Mutex::new(false);

    fn avisar(janela: &WebviewWindow, a: Area) {
        let dentro = PONTEIRO.lock().unwrap().is_some_and(|(x, y)| {
            x >= a.x && x <= a.x + a.w && y >= a.y - 2.0 && y <= a.y + a.h
        });
        let mut antes = DENTRO.lock().unwrap();
        if *antes != dentro {
            *antes = dentro;
            let _ = janela.emit("eva-mouse", dentro);
        }
    }

    /// Entrar/sair do mouse direto do GTK. O WebKit não serve pra isso: no XWayland, a
    /// saída não vira "mouseleave" na página e ele ainda repete movimentos na posição antiga.
    pub fn vigiar_ponteiro(janela: &WebviewWindow, area: AreaShared) {
        // Janela nova: esquece o que valia pra anterior.
        *PONTEIRO.lock().unwrap() = None;
        *DENTRO.lock().unwrap() = false;
        let j = janela.clone();
        let _ = janela.with_webview(move |w| {
            let web = w.inner();
            // A escala da tela pode mudar com a Eva aberta (no login, o GNOME aplica a escala
            // um pouco depois). O recorte e a posição ficariam no tamanho antigo: refaz os dois.
            let (j_escala, area_escala) = (j.clone(), area.clone());
            web.connect_scale_factor_notify(move |_| {
                let (j, area) = (j_escala.clone(), area_escala.clone());
                anotar(&j, "escala mudou");
                gtk::glib::timeout_add_local_once(Duration::from_millis(300), move || {
                    remapear(&j, move |j| {
                        let a = *area.lock().unwrap();
                        recortar(j, a);
                        let _ = super::posicionar(j);
                    });
                });
            });
            web.add_events(
                gtk::gdk::EventMask::ENTER_NOTIFY_MASK
                    | gtk::gdk::EventMask::LEAVE_NOTIFY_MASK
                    | gtk::gdk::EventMask::POINTER_MOTION_MASK,
            );
            let mudou = move |pos: Option<(f64, f64)>| {
                *PONTEIRO.lock().unwrap() = pos;
                let a = *area.lock().unwrap();
                avisar(&j, a);
            };
            let entrou = mudou.clone();
            web.connect_enter_notify_event(move |_, e| {
                entrou(Some(e.position()));
                gtk::glib::Propagation::Proceed
            });
            let moveu = mudou.clone();
            web.connect_motion_notify_event(move |_, e| {
                moveu(Some(e.position()));
                gtk::glib::Propagation::Proceed
            });
            web.connect_leave_notify_event(move |_, e| {
                // Clicar também gera um "saiu" (captura do mouse): esse não conta.
                if e.mode() == gtk::gdk::CrossingMode::Normal
                    && e.detail() != gtk::gdk::NotifyType::Inferior
                {
                    mudou(None);
                }
                gtk::glib::Propagation::Proceed
            });
        });
    }

    /// No GNOME com Wayland, um app não pode escolher onde a janela fica, nem saber
    /// onde está o mouse, nem ter tecla global. Rodando pelo XWayland (modo de
    /// compatibilidade X11, que vem no Ubuntu) a Eva consegue ficar no topo e no centro.
    /// Quem quiser testar o Wayland puro pode abrir com GDK_BACKEND=wayland.
    pub fn preparar_ambiente() {
        let tem = |v: &str| std::env::var_os(v).is_some_and(|x| !x.is_empty());
        if tem("WAYLAND_DISPLAY") && !tem("GDK_BACKEND") {
            std::env::set_var("GDK_BACKEND", "x11");
        }
        // Só em NVIDIA: evita janela em branco no XWayland. Nas outras placas fica ligado, senão
        // o WebKit do AppImage não apaga o quadro anterior e a animação deixa uma mancha preta.
        let nvidia = std::path::Path::new("/proc/driver/nvidia/version").exists();
        if nvidia && !tem("WEBKIT_DISABLE_DMABUF_RENDERER") {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }

    /// Só a área visível da Eva recebe cliques; o resto da janela deixa passar.
    /// (No Windows o Rust liga/desliga os cliques da janela inteira conforme o cursor;
    /// no Linux o próprio sistema recorta, o que funciona mesmo sem saber onde o mouse está.)
    pub fn aplicar_regiao(janela: &WebviewWindow, a: Area) {
        let j = janela.clone();
        let _ = janela.run_on_main_thread(move || {
            recortar(&j, a);
            // Se a Eva encolheu e o mouse ficou de fora, isso também conta como sair.
            avisar(&j, a);
        });
    }

    // ---- Login no GNOME ----
    // Logo depois do login o GNOME ainda vai aplicar a escala da tela. Uma janela criada
    // antes disso aparece, mas o GNOME não entrega mais o mouse a ela: o clique atravessa.
    // Por isso a Central espera a tela parar de mudar e, se a escala mudar com uma janela
    // aberta, ela é escondida e mostrada de novo (o GNOME passa a tratá-la como nova).

    /// A tela como o GTK vê agora: escala e tamanho do monitor principal.
    fn tela() -> Option<(i32, i32, i32)> {
        let d = gtk::gdk::Display::default()?;
        let m = d.primary_monitor().or_else(|| d.monitor(0))?;
        let g = m.geometry();
        Some((m.scale_factor(), g.width(), g.height()))
    }

    /// A sessão gráfica começou há pouco? (idade do soquete do Wayland)
    fn login_recente() -> bool {
        let (Ok(pasta), Ok(nome)) = (std::env::var("XDG_RUNTIME_DIR"), std::env::var("WAYLAND_DISPLAY")) else {
            return false;
        };
        std::fs::metadata(std::path::Path::new(&pasta).join(nome))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|idade| idade < Duration::from_secs(120))
    }

    /// Abre a Central; num login recente, só depois de a tela ficar parada.
    /// Chamar na thread principal (GTK).
    pub fn abrir_quando_pronta(app: &AppHandle) {
        let abrir = |app: &AppHandle| {
            if let Err(e) = crate::central::abrir(app) {
                eprintln!("[eva] central: {e}");
            }
        };
        if !login_recente() {
            escrever(&format!("abriu direto tela={:?}", tela()));
            return abrir(app);
        }
        const PARADA: Duration = Duration::from_secs(4);
        const LIMITE: Duration = Duration::from_secs(30);
        let (app, inicio) = (app.clone(), Instant::now());
        let mut ultima = tela();
        let mut desde = Instant::now();
        escrever(&format!("login: esperando, tela={ultima:?}"));
        gtk::glib::timeout_add_local(Duration::from_millis(250), move || {
            // Você abriu pelo menu enquanto esperava.
            if app.get_webview_window("central").is_some() {
                return gtk::glib::ControlFlow::Break;
            }
            let agora = tela();
            if agora != ultima {
                escrever(&format!("login: tela mudou {ultima:?} -> {agora:?}"));
                ultima = agora;
                desde = Instant::now();
            }
            if (agora.is_some() && desde.elapsed() >= PARADA) || inicio.elapsed() >= LIMITE {
                escrever(&format!("login: abriu após {:.1}s tela={agora:?}", inicio.elapsed().as_secs_f32()));
                abrir(&app);
                return gtk::glib::ControlFlow::Break;
            }
            gtk::glib::ControlFlow::Continue
        });
    }

    /// Central: se a escala mudar com ela aberta, some e volta já centralizada.
    pub fn vigiar_escala_central(janela: &WebviewWindow) {
        let j = janela.clone();
        let _ = janela.with_webview(move |w| {
            let web = w.inner();
            anotar(&j, "central criada");
            let depois = j.clone();
            gtk::glib::timeout_add_local_once(Duration::from_secs(3), move || anotar(&depois, "central 3s"));
            web.connect_scale_factor_notify(move |_| {
                anotar(&j, "escala mudou");
                let j = j.clone();
                gtk::glib::timeout_add_local_once(Duration::from_millis(300), move || {
                    remapear(&j, |j| {
                        let _ = j.center();
                    });
                });
            });
        });
    }

    /// Esconde e mostra a janela, ajustando a posição no meio. Só se ela estiver visível.
    fn remapear(j: &WebviewWindow, ajustar: impl FnOnce(&WebviewWindow) + 'static) {
        let Ok(g) = j.gtk_window() else { return };
        let g = g.upcast::<gtk::Widget>();
        if !g.is_visible() {
            return ajustar(j);
        }
        g.hide();
        let j = j.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(250), move || {
            g.show();
            ajustar(&j);
            anotar(&j, "remapeada");
        });
    }

    fn anotar(j: &WebviewWindow, o_que: &str) {
        escrever(&format!("{} {o_que}: tela={:?} posicao={:?}", j.label(), tela(), j.outer_position().ok().map(|p| (p.x, p.y))));
    }

    /// Registro pequeno, só no seu PC: ~/.config/com.myeva.app/janelas.log
    fn escrever(texto: &str) {
        use std::io::Write;
        let caminho = crate::ponte::arquivo_ponte().with_file_name("janelas.log");
        if let Some(p) = caminho.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let grande = std::fs::metadata(&caminho).is_ok_and(|m| m.len() > 64 * 1024);
        let hora = gtk::glib::DateTime::now_local().ok().and_then(|d| d.format("%F %T").ok()).unwrap_or_default();
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).write(true).append(!grande).truncate(grande).open(caminho) {
            let _ = writeln!(f, "{hora} v{} {texto}", env!("CARGO_PKG_VERSION"));
        }
    }

    /// Chamar na thread principal (GTK).
    fn recortar(j: &WebviewWindow, a: Area) {
        let Ok(gtk_janela) = j.gtk_window() else { return };
        let y0 = (a.y - 2.0).max(0.0);
        let retangulo = gtk::cairo::RectangleInt::new(
            a.x.floor() as i32,
            y0.floor() as i32,
            a.w.ceil().max(0.0) as i32,
            (a.y + a.h - y0).ceil().max(0.0) as i32,
        );
        let regiao = gtk::cairo::Region::create_rectangle(&retangulo);
        gtk_janela.input_shape_combine_region(Some(&regiao));
    }
}

/// Vigia o cursor: quando entra/sai da área da Eva, liga/desliga os cliques
/// e avisa a interface (assim a alcinha funciona mesmo "transparente").
fn vigiar_mouse(janela: WebviewWindow, area: AreaShared) {
    thread::spawn(move || {
        let mut dentro_antes: Option<bool> = None;
        let mut ultimo_cursor = (-1e9_f64, -1e9_f64);
        loop {
            thread::sleep(Duration::from_millis(40));
            // A Eva foi desligada: esta janela não existe mais.
            if janela.app_handle().get_webview_window("main").is_none() {
                return;
            }
            let (Ok(cursor), Ok(pos), Ok(escala)) =
                (janela.cursor_position(), janela.outer_position(), janela.scale_factor())
            else {
                continue;
            };
            let a = *area.lock().unwrap();
            let x = (cursor.x - pos.x as f64) / escala;
            let y = (cursor.y - pos.y as f64) / escala;
            // Posição do cursor pros olhos da Eva (só quando mexe).
            if (x - ultimo_cursor.0).abs() > 0.5 || (y - ultimo_cursor.1).abs() > 0.5 {
                ultimo_cursor = (x, y);
                let _ = janela.emit("eva-cursor", (x, y));
            }

            // No Linux quem avisa entrar/sair é o GTK (veja linux::vigiar_ponteiro).
            if cfg!(target_os = "linux") {
                continue;
            }
            let dentro = x >= a.x && x <= a.x + a.w && y >= a.y - 2.0 && y <= a.y + a.h;

            if dentro_antes != Some(dentro) {
                dentro_antes = Some(dentro);
                let _ = janela.set_ignore_cursor_events(!dentro);
                let _ = janela.emit("eva-mouse", dentro);
            }
        }
    });
}

/// Cola a Eva no topo, centralizada na tela principal.
fn posicionar(janela: &WebviewWindow) -> tauri::Result<()> {
    let Ok(Some(monitor)) = janela.primary_monitor() else { return Ok(()) };
    let tela = monitor.size();
    let origem = monitor.position();
    // No Linux o tamanho lido na abertura não é confiável (a janela ainda está se formando):
    // usa a largura do tauri.conf.json (700) na escala do monitor.
    #[cfg(target_os = "linux")]
    let largura = (700.0 * monitor.scale_factor()).round() as i32;
    #[cfg(not(target_os = "linux"))]
    let largura = janela.outer_size()?.width as i32;
    let x = origem.x + (tela.width as i32 - largura) / 2;
    // No Linux, a barra do topo do GNOME fica por cima de qualquer janela:
    // a Eva gruda logo abaixo dela (a "área de trabalho" já desconta a barra).
    #[cfg(target_os = "linux")]
    let topo = monitor.work_area().position.y.max(origem.y);
    #[cfg(not(target_os = "linux"))]
    let topo = origem.y;
    janela.set_position(PhysicalPosition::new(x, topo))
}

/// Deixa a janela da Eva (recém-criada) pronta: posição, cliques e cursor.
fn preparar_eva(app: &AppHandle, janela: &WebviewWindow) {
    let area = app.state::<AreaShared>().inner().clone();
    let _ = posicionar(janela);
    // No Linux o gerenciador de janelas ainda pode mexer nela logo depois de abrir.
    #[cfg(target_os = "linux")]
    {
        let j = janela.clone();
        thread::spawn(move || {
            for espera in [300, 1200] {
                thread::sleep(Duration::from_millis(espera));
                let _ = posicionar(&j);
            }
        });
        linux::vigiar_ponteiro(janela, area.clone());
    }
    vigiar_mouse(janela.clone(), area);
}

/// Controle, música e ponte. Começam uma vez só, quando a Eva liga pela primeira vez.
fn iniciar_servicos(app: &AppHandle) {
    ds4::iniciar(app.clone(), app.state::<ds4::Shared>().inner().clone());
    let canal = musica::iniciar(app.clone(), app.state::<musica::Shared>().inner().clone());
    app.manage::<musica::Canal>(Mutex::new(canal));
    ponte::iniciar(app.clone(), app.state::<ponte::PonteShared>().inner().clone());
}

fn main() {
    let estado_ds4: ds4::Shared = Arc::new(Mutex::new(ds4::Ds4Status::default()));
    let area: AreaShared = Arc::new(Mutex::new(Area::default()));
    let estado_musica: musica::Shared = Arc::new(Mutex::new(musica::Estado::default()));
    let interfone: ponte::PonteShared = Arc::new(ponte::Ponte::default());

    #[cfg(target_os = "linux")]
    linux::preparar_ambiente();

    let construtor = tauri::Builder::default();
    // Uma Eva só: abrir o programa de novo traz a Central. No Linux, `my-eva --alternar`
    // (dá pra ligar num atalho de teclado do GNOME) mostra/esconde a Eva ligada.
    let construtor = construtor.plugin(tauri_plugin_single_instance::init(|app, args, _pasta| {
        if args.iter().any(|a| a == "--alternar") && central::ligada(app) {
            let _ = app.emit("eva-atalho", ());
        } else {
            central::abrir_em_thread(app);
        }
    }));

    construtor
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _atalho, evento| {
                    if matches!(evento.state(), ShortcutState::Pressed) {
                        let _ = app.emit("eva-atalho", ());
                    }
                })
                .build(),
        )
        // Iniciar junto com o sistema: no Windows, entrada em HKCU\...\Run;
        // no Linux, um atalho em ~/.config/autostart. Só pro seu usuário.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(estado_ds4.clone())
        .manage(area.clone())
        .manage(estado_musica.clone())
        .manage(interfone.clone())
        .manage(central::Controle::default())
        .manage(atualizacao::Atualizacao::default())
        .invoke_handler(tauri::generate_handler![
            ds4_status,
            definir_area,
            musica_status,
            musica_acao,
            claude_responder,
            central::central_estado,
            central::central_ligar,
            central::central_desligar,
            central::central_ver_pedido,
            central::central_fechar,
            central::central_inicio,
            atualizacao::atualizacao_estado,
            atualizacao::atualizacao_definir,
            atualizacao::atualizacao_procurar,
            atualizacao::atualizacao_instalar
        ])
        .setup(move |app| {
            // Iniciar com o sistema: liga sozinho só na primeira vez da versão instalada.
            // Se você desligar pelo menu, ela respeita e não liga de novo.
            #[cfg(not(debug_assertions))]
            {
                let marca = ponte::arquivo_ponte().with_file_name("inicio-automatico-configurado");
                if !marca.exists() {
                    let _ = app.autolaunch().enable();
                    // Na primeira abertura a pasta pode ainda não existir (a ponte só a cria depois).
                    // Sem ela a marca não era gravada, e a segunda abertura religava o início
                    // automático mesmo que você já tivesse desligado.
                    if let Some(pasta) = marca.parent() {
                        let _ = std::fs::create_dir_all(pasta);
                    }
                    let _ = std::fs::write(&marca, "1");
                }
            }
            let inicia_sozinha = app.autolaunch().is_enabled().unwrap_or(false);

            // Ícone na bandeja (perto do relógio).
            let auto = CheckMenuItem::with_id(
                app,
                "auto",
                if cfg!(windows) { "Iniciar com o Windows" } else { "Iniciar com o sistema" },
                true,
                inicia_sozinha,
                None::<&str>,
            )?;
            let sep = PredefinedMenuItem::separator(app)?;
            let central = MenuItem::with_id(app, "central", "Abrir a Central", true, None::<&str>)?;
            let sair = MenuItem::with_id(app, "sair", "Sair da Eva", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&central, &auto, &sep, &sair])?;
            app.manage(central::MenuInicio(auto));
            TrayIconBuilder::with_id(central::BANDEJA)
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("My Eva")
                .menu(&menu)
                .on_menu_event(|app, evento| match evento.id.as_ref() {
                    "central" => central::abrir_em_thread(app),
                    "sair" => central::desligar_em_thread(app),
                    "auto" => {
                        let ligado = app.autolaunch().is_enabled().unwrap_or(false);
                        central::definir_inicio(app, !ligado);
                    }
                    _ => {}
                })
                .build(app)?
                // Só aparece com a Eva ligada.
                .set_visible(false)?;

            #[cfg(target_os = "linux")]
            linux::abrir_quando_pronta(app.handle());
            #[cfg(not(target_os = "linux"))]
            central::abrir(app.handle())?;
            Ok(())
        })
        .on_window_event(|janela, evento| {
            // No meio de uma atualização a Central não fecha (o arquivo pode estar pela metade).
            if let ("central", tauri::WindowEvent::CloseRequested { api, .. }) = (janela.label(), evento) {
                if janela.state::<atualizacao::Atualizacao>().trabalhando() {
                    api.prevent_close();
                }
            }
            if janela.label() == "main" && matches!(evento, tauri::WindowEvent::Destroyed) {
                central::eva_fechou(janela.app_handle());
            }
        })
        .run(tauri::generate_context!())
        .expect("erro ao iniciar a Eva");
}
