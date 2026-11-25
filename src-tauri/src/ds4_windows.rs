//! Monitor da bateria do DualShock 4 (USB ou Bluetooth) via HID.
//!
//! Como funciona:
//! - O DS4 manda "relatórios de entrada" (input reports) várias vezes por segundo.
//! - Pelo USB, o relatório 0x01 traz a bateria no byte 30.
//! - Pelo Bluetooth, o controle começa num modo simplificado (0x01) sem bateria.
//!   Pedir o feature report 0x05 (ou 0x02) faz ele trocar pro modo completo (0x11),
//!   que traz a bateria no byte 32.
//! - Nesse byte: os 4 bits baixos = nível; o bit 0x10 = cabo conectado (carregando).

use hidapi::{BusType, HidApi, HidDevice};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const SONY: u16 = 0x054C;
/// DS4 v1, DS4 v2 e o adaptador USB sem fio da Sony.
const PIDS_DS4: [u16; 3] = [0x05C4, 0x09CC, 0x0BA0];
/// UUID do serviço HID por Bluetooth, aparece no caminho do dispositivo no Windows.
const UUID_HID_BT: &str = "00001124-0000-1000-8000-00805f9b34fb";

#[derive(Clone, Serialize, Default, PartialEq, Debug)]
pub struct Ds4Status {
    pub conectado: bool,
    /// 0 a 100
    pub bateria: u8,
    pub carregando: bool,
    /// "bluetooth" ou "usb"
    pub conexao: String,
}

pub type Shared = Arc<Mutex<Ds4Status>>;

struct Controle {
    dev: HidDevice,
    bluetooth: bool,
    leituras_vazias: u32,
}

enum Leitura {
    Ok(Ds4Status),
    SemDado,
    Erro,
}

pub fn iniciar(app: AppHandle, estado: Shared) {
    thread::spawn(move || {
        let mut api = match HidApi::new() {
            Ok(a) => a,
            Err(e) => {
                eprintln!("[eva] não consegui iniciar o HID: {e}");
                return;
            }
        };
        let mut controle: Option<Controle> = None;

        loop {
            if controle.is_none() {
                controle = procurar(&mut api);
            }

            let leitura = controle.as_mut().map(ler);
            let novo = match leitura {
                None => Some(Ds4Status::default()),
                Some(Leitura::Ok(s)) => {
                    controle.as_mut().unwrap().leituras_vazias = 0;
                    Some(s)
                }
                Some(Leitura::SemDado) => {
                    let c = controle.as_mut().unwrap();
                    c.leituras_vazias += 1;
                    // Muitas leituras sem resposta = controle desligou.
                    if c.leituras_vazias >= 5 {
                        controle = None;
                        Some(Ds4Status::default())
                    } else {
                        None
                    }
                }
                Some(Leitura::Erro) => {
                    controle = None;
                    Some(Ds4Status::default())
                }
            };

            if let Some(s) = novo {
                let mut atual = estado.lock().unwrap();
                if *atual != s {
                    *atual = s.clone();
                    drop(atual);
                    #[cfg(debug_assertions)]
                    eprintln!("[eva] ds4: {s:?}");
                    let _ = app.emit("ds4-status", s);
                }
            }

            let espera = if controle.is_some() { 2000 } else { 3000 };
            thread::sleep(Duration::from_millis(espera));
        }
    });
}

fn procurar(api: &mut HidApi) -> Option<Controle> {
    if api.refresh_devices().is_err() {
        return None;
    }
    let api: &HidApi = api;
    for info in api.device_list() {
        if info.vendor_id() != SONY || !PIDS_DS4.contains(&info.product_id()) {
            continue;
        }
        let caminho = info.path().to_string_lossy().to_lowercase();
        let bluetooth = matches!(info.bus_type(), BusType::Bluetooth) || caminho.contains(UUID_HID_BT);

        if let Ok(dev) = info.open_device(api) {
            if bluetooth {
                ativar_modo_completo(&dev);
            }
            return Some(Controle { dev, bluetooth, leituras_vazias: 0 });
        }
    }
    None
}

/// No Bluetooth, pedir esse feature report faz o DS4 mandar o relatório completo (0x11).
fn ativar_modo_completo(dev: &HidDevice) {
    let mut buf = [0u8; 600];
    buf[0] = 0x05;
    if dev.get_feature_report(&mut buf).is_err() {
        let mut buf = [0u8; 600];
        buf[0] = 0x02;
        let _ = dev.get_feature_report(&mut buf);
    }
}

fn ler(c: &mut Controle) -> Leitura {
    let mut buf = [0u8; 600];
    let n = match c.dev.read_timeout(&mut buf, 500) {
        Err(_) => return Leitura::Erro,
        Ok(0) => return Leitura::SemDado,
        Ok(n) => n,
    };

    let indice = if c.bluetooth {
        if buf[0] != 0x11 {
            // Ainda no modo simplificado: pede de novo.
            ativar_modo_completo(&c.dev);
            return Leitura::SemDado;
        }
        32
    } else {
        if buf[0] != 0x01 {
            return Leitura::SemDado;
        }
        30
    };
    if n <= indice {
        return Leitura::SemDado;
    }

    let byte = buf[indice];
    let nivel = (byte & 0x0F) as u32;
    let carregando = byte & 0x10 != 0;
    // Sem cabo o nível vai de 0 a 8; com cabo vai até 11 (11 = cheio).
    let maximo = if carregando { 11 } else { 8 };
    let bateria = ((nivel * 100) / maximo).min(100) as u8;

    Leitura::Ok(Ds4Status {
        conectado: true,
        bateria,
        carregando,
        conexao: if c.bluetooth { "bluetooth" } else { "usb" }.into(),
    })
}
