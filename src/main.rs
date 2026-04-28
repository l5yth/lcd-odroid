// Copyright 2026 l5y
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Entry point: load `config.toml`, initialise the LCD, dispatch to the
//! mode-specific runner. All real work lives in the `runner` submodules and
//! the `lcd_odroid` library; this file only wires them together.

mod hardware;
mod runner;

use std::fs;

use lcd_odroid::info;

/// Path to the TOML configuration file, relative to the working directory.
const CONFIG_PATH: &str = "config.toml";

/// Reads an optional string field from `cfg`, returning `default` if it is
/// absent. Errors if the field is present but not a string.
fn cfg_str(
    cfg: &toml::Value,
    key: &str,
    default: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    match cfg.get(key) {
        None => Ok(default.to_string()),
        Some(v) => v
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| format!("config.toml: '{key}' must be a string").into()),
    }
}

/// Reads a required string field from `cfg`. Errors if the field is missing
/// or is not a string.
fn cfg_required(cfg: &toml::Value, key: &str) -> Result<String, Box<dyn std::error::Error>> {
    cfg.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("config.toml: missing '{key}'").into())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg: toml::Value = toml::from_str(&fs::read_to_string(CONFIG_PATH)?)?;
    let layer = cfg
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or("config.toml: missing or non-string 'type' field")?;
    info!("loaded {} (layer={})", CONFIG_PATH, layer);

    let mut lcd = hardware::init_lcd()?;

    match layer {
        "hostname" => runner::hostname::run(&mut lcd),
        "execution" => {
            let http = cfg_str(&cfg, "el_http_url", runner::execution::HTTP_URL_DEFAULT)?;
            let ws = cfg_str(&cfg, "el_ws_url", runner::execution::WS_URL_DEFAULT)?;
            runner::execution::run(&mut lcd, &http, &ws)
        }
        "consensus" => {
            let cl_url = cfg_str(&cfg, "cl_url", runner::consensus::HTTP_URL_DEFAULT)?;
            runner::consensus::run(&mut lcd, &cl_url)
        }
        "bitcoin" => {
            let btc_url = cfg_str(&cfg, "btc_url", runner::bitcoin::HTTP_URL_DEFAULT)?;
            let user = cfg_required(&cfg, "rpcuser")?;
            let pass = cfg_required(&cfg, "rpcpassword")?;
            runner::bitcoin::run(&mut lcd, &btc_url, &user, &pass)
        }
        other => Err(format!("config.toml: unsupported type {other:?}").into()),
    }
}
