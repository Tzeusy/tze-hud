//! Deterministic Layer-3 token-footprint calibration for canonical MCP flows.

#[cfg(feature = "headless")]
mod calibration {
    use serde::{Deserialize, Serialize};
    use std::collections::{BTreeMap, HashMap};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use tokio::sync::mpsc;
    use tze_hud_runtime::headless::{HeadlessConfig, HeadlessRuntime};
    use tze_hud_runtime::portal_projection_driver::{InProcessPortalDriver, PortalOp};
    use tze_hud_runtime::threads::{ShutdownReason, ShutdownToken};
    use tze_hud_runtime::{McpServerConfig, start_mcp_http_server};
    use tze_hud_scene::types::{
        ContentionPolicy, GeometryPolicy, RenderingPolicy, SceneId, WidgetDefinition,
        WidgetInstance, WidgetParamType, WidgetParameterDeclaration, WidgetParameterValue,
    };

    const PSK: &str = "token-calibration-resident-psk";
    const LEGACY_FLOW_VERSION: u32 = 1;
    const COMBINED_PORTAL_FLOW_VERSION: u32 = 2;
    const VOCAB_FINGERPRINT: &str =
        "sha256:446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d";

    /// The approved v1 baseline remains the default. The combined mode is an
    /// isolated, deliberately unapproved v2 candidate that the fail-closed v1
    /// checker never reads as its baseline.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum CalibrationMode {
        LegacyV1,
        CombinedCandidateV2,
    }

    impl CalibrationMode {
        fn parse(value: &str) -> Self {
            match value {
                "legacy-v1" => Self::LegacyV1,
                "combined-candidate-v2" => Self::CombinedCandidateV2,
                _ => {
                    panic!("invalid --mode {value:?}; expected legacy-v1 or combined-candidate-v2")
                }
            }
        }

        fn python_value(self) -> &'static str {
            match self {
                Self::LegacyV1 => "legacy-v1",
                Self::CombinedCandidateV2 => "combined-candidate-v2",
            }
        }

        fn flow_version(self, flow_name: &str) -> u32 {
            match (self, flow_name) {
                (Self::CombinedCandidateV2, "portal_projection") => COMBINED_PORTAL_FLOW_VERSION,
                _ => LEGACY_FLOW_VERSION,
            }
        }
    }

    #[derive(Debug)]
    struct CalibrationArgs {
        output_path: PathBuf,
        mode: CalibrationMode,
    }

    #[derive(Debug, Deserialize)]
    struct DriverOutput {
        transactions: Vec<Transaction>,
    }

    #[derive(Debug, Deserialize)]
    struct Transaction {
        method: String,
        request_body: String,
        response_body: String,
    }

    #[derive(Clone, Debug, Serialize)]
    struct Counts {
        bytes: usize,
        tokens: usize,
    }

    #[derive(Clone, Debug, Serialize)]
    struct OperationMeasurement {
        request: Counts,
        response: Counts,
        total: Counts,
    }

    #[derive(Debug, Serialize)]
    struct FlowMeasurement {
        flow_version: u32,
        flow_fingerprint: String,
        operations: BTreeMap<String, OperationMeasurement>,
        total: Counts,
    }

    #[derive(Debug, Serialize)]
    struct TokenizerIdentity {
        name: &'static str,
        implementation: &'static str,
        version: &'static str,
        vocab_fingerprint: &'static str,
        counting_policy: &'static str,
    }

    #[derive(Debug, Serialize)]
    struct CalibrationOutput {
        schema_version: u32,
        runtime_build: &'static str,
        tokenizer: TokenizerIdentity,
        fixture_fingerprint: String,
        flows: BTreeMap<String, FlowMeasurement>,
    }

    fn headless_config() -> HeadlessConfig {
        HeadlessConfig {
            width: 1920,
            height: 1080,
            grpc_port: 0,
            bind_all_interfaces: false,
            psk: PSK.to_string(),
            config_toml: Some(String::new()),
        }
    }

    async fn prepare_scene(runtime: &HeadlessRuntime) {
        let state = runtime.shared_state().lock().await;
        let mut scene = state.scene.lock().await;
        scene.zone_registry = tze_hud_scene::types::ZoneRegistry::with_defaults();
        let tab_id = scene
            .create_tab("Token Calibration", 0)
            .expect("create calibration tab");
        scene.widget_registry.register_definition(WidgetDefinition {
            id: "token-calibration-gauge".to_string(),
            name: "Token Calibration Gauge".to_string(),
            description: "Deterministic one-parameter calibration widget".to_string(),
            parameter_schema: vec![WidgetParameterDeclaration {
                name: "level".to_string(),
                param_type: WidgetParamType::F32,
                default_value: WidgetParameterValue::F32(0.0),
                constraints: None,
            }],
            layers: vec![],
            default_geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 0.2,
                height_pct: 0.1,
            },
            default_rendering_policy: RenderingPolicy::default(),
            default_contention_policy: ContentionPolicy::LatestWins,
            max_publishers: WidgetDefinition::default_max_publishers(),
            ephemeral: false,
            hover_behavior: None,
        });
        scene.widget_registry.register_instance(WidgetInstance {
            id: SceneId::new(),
            widget_type_name: "token-calibration-gauge".to_string(),
            tab_id,
            geometry_override: None,
            contention_override: None,
            instance_name: "token-calibration-gauge".to_string(),
            current_params: HashMap::from([("level".to_string(), WidgetParameterValue::F32(0.0))]),
        });
    }

    async fn scene_handle(
        runtime: &HeadlessRuntime,
    ) -> std::sync::Arc<tokio::sync::Mutex<tze_hud_scene::graph::SceneGraph>> {
        let state = runtime.shared_state().lock().await;
        state.scene.clone()
    }

    fn spawn_portal_driver(
        mut rx: mpsc::UnboundedReceiver<PortalOp>,
        mode: CalibrationMode,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut driver = InProcessPortalDriver::new();
            let mut input_injected = false;
            while let Some(op) = rx.recv().await {
                let is_attach = matches!(&op, PortalOp::Attach { .. });
                let is_publish = matches!(
                    &op,
                    PortalOp::PublishOutput { .. } | PortalOp::PublishOutputWithPendingInput { .. }
                );
                driver.dispatch_portal_op(op);
                // The v1 calibration injects after its publish, so that the
                // response stays body-identical to the approved legacy
                // acknowledgement and the existing explicit poll receives the
                // fixture input. The v2 candidate injects after attach, making
                // that same input available to the publish-response piggyback.
                let should_inject = match mode {
                    CalibrationMode::LegacyV1 => is_publish,
                    CalibrationMode::CombinedCandidateV2 => is_attach,
                };
                if should_inject && !input_injected {
                    driver
                        .authority_mut()
                        .enqueue_input(
                            "token-calibration-portal",
                            "canonical-input-0001",
                            "Canonical HUD-originated input.".to_string(),
                            1_700_000_000_100_000,
                            9_000_000_000_000_000,
                            None,
                        )
                        .expect("inject canonical portal input");
                    input_injected = true;
                }
            }
        })
    }

    fn run_python_driver(address: SocketAddr, mode: CalibrationMode) -> DriverOutput {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let driver = root.join("examples/benchmark/token_footprint_flow.py");
        let portal_client = root.join(".claude/skills/hud-projection/scripts/portal_client.py");
        let output = Command::new("python3")
            .arg(driver)
            .env("HUD_MCP_URL", format!("http://{address}/mcp"))
            .env("HUD_PSK", PSK)
            .env("PORTAL_CLIENT_PATH", portal_client)
            .env("TOKEN_FOOTPRINT_MODE", mode.python_value())
            .output()
            .expect("launch token-footprint Python flow driver");
        assert!(
            output.status.success(),
            "Python flow driver failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("parse Python flow-driver output")
    }

    fn count(body: &str, bpe: &tiktoken_rs::CoreBPE) -> Counts {
        Counts {
            bytes: body.len(),
            tokens: bpe.encode_with_special_tokens(body).len(),
        }
    }

    fn sum_counts(left: &Counts, right: &Counts) -> Counts {
        Counts {
            bytes: left.bytes + right.bytes,
            tokens: left.tokens + right.tokens,
        }
    }

    fn fingerprint(parts: impl IntoIterator<Item = String>) -> String {
        let mut hasher = blake3::Hasher::new();
        for part in parts {
            hasher.update(&(part.len() as u64).to_le_bytes());
            hasher.update(part.as_bytes());
        }
        format!("blake3:{}", hasher.finalize().to_hex())
    }

    fn build_output(driver: DriverOutput, mode: CalibrationMode) -> CalibrationOutput {
        let bpe = tiktoken_rs::o200k_base().expect("load bundled o200k_base vocabulary");
        let mut grouped: BTreeMap<String, Vec<Transaction>> = BTreeMap::new();
        for transaction in driver.transactions {
            let flow = match transaction.method.as_str() {
                "publish_to_zone" => "publish_to_zone",
                "publish_to_widget" => "publish_to_widget",
                "portal_projection_attach"
                | "portal_projection_publish"
                | "portal_projection_get_pending_input"
                | "portal_projection_acknowledge_input" => "portal_projection",
                method => panic!("unexpected canonical method: {method}"),
            };
            grouped
                .entry(flow.to_string())
                .or_default()
                .push(transaction);
        }

        let mut fixture_parts = Vec::new();
        let mut flows = BTreeMap::new();
        for (flow_name, transactions) in grouped {
            let mut operations = BTreeMap::new();
            let mut flow_total = Counts {
                bytes: 0,
                tokens: 0,
            };
            let mut flow_parts = Vec::new();
            for transaction in transactions {
                let request = count(&transaction.request_body, &bpe);
                let response = count(&transaction.response_body, &bpe);
                let total = sum_counts(&request, &response);
                flow_total = sum_counts(&flow_total, &total);
                flow_parts.extend([
                    transaction.method.clone(),
                    transaction.request_body.clone(),
                    transaction.response_body.clone(),
                ]);
                fixture_parts.extend([
                    transaction.method.clone(),
                    transaction.request_body.clone(),
                    transaction.response_body.clone(),
                ]);
                operations.insert(
                    transaction.method,
                    OperationMeasurement {
                        request,
                        response,
                        total,
                    },
                );
            }
            let flow_version = mode.flow_version(&flow_name);
            flows.insert(
                flow_name,
                FlowMeasurement {
                    flow_version,
                    flow_fingerprint: fingerprint(flow_parts),
                    operations,
                    total: flow_total,
                },
            );
        }

        CalibrationOutput {
            schema_version: 1,
            runtime_build: env!("CARGO_PKG_VERSION"),
            tokenizer: TokenizerIdentity {
                name: "o200k_base",
                implementation: "tiktoken-rs",
                version: "0.12.0",
                vocab_fingerprint: VOCAB_FINGERPRINT,
                counting_policy: "each canonical JSON-RPC body independently; UTF-8 bytes; encode_with_special_tokens; operation and flow totals are integer sums",
            },
            fixture_fingerprint: fingerprint(fixture_parts),
            flows,
        }
    }

    fn parse_args() -> CalibrationArgs {
        let mut output_path = None;
        let mut mode = CalibrationMode::LegacyV1;
        let mut args = std::env::args().skip(1);

        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--output" => {
                    let path = args
                        .next()
                        .unwrap_or_else(|| panic!("--output requires a path"));
                    output_path = Some(PathBuf::from(path));
                }
                "--mode" => {
                    let value = args.next().unwrap_or_else(|| {
                        panic!("--mode requires legacy-v1 or combined-candidate-v2")
                    });
                    mode = CalibrationMode::parse(&value);
                }
                _ => panic!(
                    "usage: token_footprint_calibration --output <path> [--mode legacy-v1|combined-candidate-v2]"
                ),
            }
        }

        CalibrationArgs {
            output_path: output_path.unwrap_or_else(|| {
                panic!(
                    "usage: token_footprint_calibration --output <path> [--mode legacy-v1|combined-candidate-v2]"
                )
            }),
            mode,
        }
    }

    pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let args = parse_args();
        let runtime = HeadlessRuntime::new(headless_config()).await?;
        prepare_scene(&runtime).await;
        let scene = scene_handle(&runtime).await;
        let (portal_tx, portal_rx) = mpsc::unbounded_channel();
        let portal_task = spawn_portal_driver(portal_rx, args.mode);
        let shutdown = ShutdownToken::new();
        let config = McpServerConfig {
            bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            psk: PSK.to_string(),
            resident_principal: Some(PSK.to_string()),
        };
        let (server_task, address) = start_mcp_http_server(
            scene,
            config,
            shutdown.clone(),
            None,
            Some(portal_tx.clone()),
        )
        .await?;
        let mode = args.mode;
        let driver_output =
            tokio::task::spawn_blocking(move || run_python_driver(address, mode)).await?;
        let output = build_output(driver_output, args.mode);
        if let Some(parent) = args.output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&args.output_path, serde_json::to_vec_pretty(&output)?)?;
        shutdown.trigger(ShutdownReason::Clean);
        server_task.await?;
        drop(portal_tx);
        portal_task.await?;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn summing_counts_is_exact_integer_addition() {
            assert_eq!(
                sum_counts(
                    &Counts {
                        bytes: 1,
                        tokens: 2
                    },
                    &Counts {
                        bytes: 3,
                        tokens: 5
                    }
                )
                .bytes,
                4
            );
            assert_eq!(
                sum_counts(
                    &Counts {
                        bytes: 1,
                        tokens: 2
                    },
                    &Counts {
                        bytes: 3,
                        tokens: 5
                    }
                )
                .tokens,
                7
            );
        }

        #[test]
        fn length_prefixes_make_fingerprints_unambiguous() {
            assert_ne!(
                fingerprint(["ab".to_string(), "c".to_string()]),
                fingerprint(["a".to_string(), "bc".to_string()])
            );
        }

        #[test]
        fn combined_candidate_versions_only_the_changed_portal_flow() {
            assert_eq!(
                CalibrationMode::CombinedCandidateV2.flow_version("portal_projection"),
                COMBINED_PORTAL_FLOW_VERSION
            );
            assert_eq!(
                CalibrationMode::CombinedCandidateV2.flow_version("publish_to_zone"),
                LEGACY_FLOW_VERSION
            );
            assert_eq!(
                CalibrationMode::LegacyV1.flow_version("portal_projection"),
                LEGACY_FLOW_VERSION
            );
        }
    }
}

#[cfg(feature = "headless")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    calibration::run().await
}

#[cfg(not(feature = "headless"))]
fn main() {
    eprintln!("token_footprint_calibration requires --features headless");
    std::process::exit(2);
}
