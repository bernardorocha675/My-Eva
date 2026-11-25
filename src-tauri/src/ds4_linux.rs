//! Monitor da bateria do DualShock 4 no Linux.
//!
//! Aqui não precisa falar HID com o controle: o próprio kernel (drivers
//! hid-playstation ou hid-sony) já lê a bateria e mostra em
//! /sys/class/power_supply/, igual à bateria de um notebook:
//!   ps-controller-battery-<mac>     (hid-playstation, kernels novos)
//!   sony_controller_battery_<mac>   (hid-sony, kernels antigos)
//! Cada pasta tem `capacity` (0–100) e `status` (Charging / Discharging / Full).
//! Ler esses arquivos não precisa de permissão especial nem de regra udev.

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// DS4 v1, DS4 v2 e o adaptador USB sem fio da Sony (mesma lista do Windows).
const PIDS_DS4: [u32; 3] = [0x05C4, 0x09CC, 0x0BA0];
const SONY: u32 = 0x054C;
const PASTA: &str = "/sys/class/power_supply";

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

pub fn iniciar(app: AppHandle, estado: Shared) {
    thread::spawn(move || loop {
        let s = ler().unwrap_or_default();
        let espera = if s.conectado { 2000 } else { 3000 };
        let mut atual = estado.lock().unwrap();
        if *atual != s {
            *atual = s.clone();
            drop(atual);
            #[cfg(debug_assertions)]
            eprintln!("[eva] ds4: {s:?}");
            let _ = app.emit("ds4-status", s);
        } else {
            drop(atual);
        }
        thread::sleep(Duration::from_millis(espera));
    });
}

fn ler() -> Option<Ds4Status> {
    for entrada in fs::read_dir(PASTA).ok()?.flatten() {
        let nome = entrada.file_name().to_string_lossy().to_lowercase();
        if !(nome.starts_with("ps-controller-battery") || nome.starts_with("sony_controller_battery")) {
            continue;
        }
        let pasta = entrada.path();
        if !eh_ds4(&pasta) {
            continue;
        }
        let Some(bateria) = ler_texto(&pasta.join("capacity")).and_then(|t| t.parse::<u8>().ok()) else {
            continue;
        };
        let status = ler_texto(&pasta.join("status")).unwrap_or_default().to_lowercase();
        // "Full" só aparece com o cabo ligado; "Charging" é carregando de fato.
        let carregando = status == "charging" || status == "full";
        return Some(Ds4Status {
            conectado: true,
            bateria: bateria.min(100),
            carregando,
            conexao: if eh_bluetooth(&pasta) { "bluetooth" } else { "usb" }.into(),
        });
    }
    None
}

/// Confere pelo HID_ID do dispositivo que é um DS4 (e não um DualSense, por exemplo).
fn eh_ds4(pasta: &Path) -> bool {
    let Some(uevent) = ler_texto(&pasta.join("device/uevent")) else { return false };
    uevent
        .lines()
        .find_map(|l| l.strip_prefix("HID_ID="))
        .map(|id| {
            // Formato: BARRAMENTO:VENDOR:PRODUTO, em hexadecimal (ex.: 0005:0000054C:000009CC).
            let partes: Vec<u32> = id.split(':').filter_map(|p| u32::from_str_radix(p, 16).ok()).collect();
            partes.len() == 3 && partes[1] == SONY && PIDS_DS4.contains(&partes[2])
        })
        .unwrap_or(false)
}

/// Barramento 0005 no HID_ID = Bluetooth; 0003 = USB.
fn eh_bluetooth(pasta: &Path) -> bool {
    if let Some(uevent) = ler_texto(&pasta.join("device/uevent")) {
        if let Some(id) = uevent.lines().find_map(|l| l.strip_prefix("HID_ID=")) {
            return id.starts_with("0005");
        }
    }
    let real: PathBuf = fs::canonicalize(pasta.join("device")).unwrap_or_default();
    let r = real.to_string_lossy();
    r.contains("bluetooth") || r.contains("/hci")
}

fn ler_texto(caminho: &Path) -> Option<String> {
    fs::read_to_string(caminho).ok().map(|t| t.trim().to_string())
}
