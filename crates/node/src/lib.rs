#![deny(clippy::all)]

use std::{
  collections::HashMap,
  fs::remove_dir_all,
  path::{Path, PathBuf},
  sync::Arc,
};

use farmfe_compiler::{update::UpdateType, Compiler};

pub mod plugin_adapters;

use farmfe_core::{
  config::{Config, Mode},
  module::ModuleId,
  relative_path::RelativePath,
};
use farmfe_toolkit::tracing_subscriber::{self, fmt, prelude::*, EnvFilter};
use napi::{bindgen_prelude::FromNapiValue, Env, JsObject, NapiRaw, Status};
use plugin_adapters::{js_plugin_adapter::JsPluginAdapter, rust_plugin_adapter::RustPluginAdapter};

#[macro_use]
extern crate napi_derive;

#[napi(object)]
pub struct JsUpdateResult {
  pub added: Vec<String>,
  pub changed: Vec<String>,
  pub removed: Vec<String>,
  pub modules: String,
  pub boundaries: HashMap<String, Vec<Vec<String>>>,
  pub dynamic_resources_map: Option<HashMap<String, Vec<Vec<String>>>>,
}

#[napi(js_name = "Compiler")]
pub struct JsCompiler {
  compiler: Compiler,
}

#[napi]
impl JsCompiler {
  #[napi(constructor)]
  pub fn new(env: Env, config: JsObject) -> napi::Result<Self> {
    let js_plugins = unsafe {
      Vec::<JsObject>::from_napi_value(
        env.raw(),
        config
          .get_named_property::<JsObject>("jsPlugins")
          .expect("jsPlugins must exist")
          .raw(),
      )
      .expect("jsPlugins should be an array of js functions")
    };

    let rust_plugins = unsafe {
      Vec::<Vec<String>>::from_napi_value(
        env.raw(),
        config
          .get_named_property::<JsObject>("rustPlugins")
          .expect("rustPlugins must exists")
          .raw(),
      )
      .expect("rustPlugins should be an array of js strings")
    };

    let config: Config = env
      .from_js_value(
        config
          .get_named_property::<JsObject>("config")
          .expect("config should exist"),
      )
      .expect("can not transform js config object to rust config");
    let mut plugins_adapters = vec![];

    for js_plugin_object in js_plugins {
      let js_plugin = Arc::new(
        JsPluginAdapter::new(&env, js_plugin_object)
          .unwrap_or_else(|e| panic!("load rust plugin error: {:?}", e)),
      ) as _;
      plugins_adapters.push(js_plugin);
    }

    for rust_plugin in rust_plugins {
      let rust_plugin_path = rust_plugin[0].clone();
      let rust_plugin_options = rust_plugin[1].clone();

      let rust_plugin = Arc::new(
        RustPluginAdapter::new(&rust_plugin_path, &config, rust_plugin_options)
          .unwrap_or_else(|e| panic!("load rust plugin error: {:?}", e)),
      ) as _;
      plugins_adapters.push(rust_plugin);
    }

    let fmt_layer = fmt::layer().with_target(false);
    let filter_layer = EnvFilter::try_from_default_env()
      .or_else(|_| EnvFilter::try_new("info"))
      .unwrap();

    tracing_subscriber::registry()
      .with(filter_layer)
      .with(fmt_layer)
      .try_init()
      .err();

    Ok(Self {
      compiler: Compiler::new(config, plugins_adapters)
        .map_err(|e| napi::Error::new(Status::GenericFailure, format!("{}", e)))?,
    })
  }

  /// async compile, return promise
  ///
  /// TODO: usage example
  #[napi]
  pub async fn compile(&self) -> napi::Result<()> {
    let context = self.compiler.context();
    let output_dir = if Path::new(&context.config.output.path).is_absolute() {
      PathBuf::from(&context.config.output.path)
    } else {
      RelativePath::new(&context.config.output.path).to_logical_path(&context.config.root)
    };

    if output_dir.exists() {
      remove_dir_all(&output_dir).map_err(|e| {
        napi::Error::new(
          Status::GenericFailure,
          format!("remove output dir error: {}", e),
        )
      })?;
    }

    self
      .compiler
      .compile()
      .map_err(|e| napi::Error::new(Status::GenericFailure, format!("{}", e)))?;

    Ok(())
  }

  /// sync compile
  #[napi]
  pub fn compile_sync(&self) -> napi::Result<()> {
    unimplemented!("sync compile is not supported yet")
  }

  /// async update, return promise
  ///
  /// TODO: usage example
  #[napi]
  pub async fn update(&self, paths: Vec<String>) -> napi::Result<JsUpdateResult> {
    // TODO transform UpdateType
    let res = self
      .compiler
      .update(
        paths
          .into_iter()
          .map(|p| (p, UpdateType::Updated))
          .collect(),
      )
      .map_err(|e| napi::Error::new(Status::GenericFailure, format!("{}", e)))?;

    Ok(JsUpdateResult {
      added: res
        .added_module_ids
        .into_iter()
        .map(|id| id.id(Mode::Development))
        .collect(),
      changed: res
        .updated_module_ids
        .into_iter()
        .map(|id| id.id(Mode::Development))
        .collect(),
      removed: res
        .removed_module_ids
        .into_iter()
        .map(|id| id.id(Mode::Development))
        .collect(),
      modules: res.resources,
      boundaries: res.boundaries,
      dynamic_resources_map: res.dynamic_resources_map.map(|dynamic_resources_map| {
        dynamic_resources_map
          .into_iter()
          .map(|(k, v)| {
            (
              k.id(self.compiler.context().config.mode.clone()),
              v.into_iter()
                .map(|(path, ty)| vec![path, ty.to_html_tag()])
                .collect(),
            )
          })
          .collect()
      }),
    })
  }

  /// sync update
  #[napi]
  pub fn update_sync(&self, paths: Vec<String>) -> napi::Result<JsUpdateResult> {
    unimplemented!("sync update");
  }

  #[napi]
  pub fn has_module(&self, resolved_path: String) -> bool {
    let context = self.compiler.context();
    let module_graph = context.module_graph.read();
    let module_id = ModuleId::new(&resolved_path, &context.config.root);

    module_graph.has_module(&module_id)
  }
}
