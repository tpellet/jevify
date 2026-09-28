use crate::cli::Shell;
use crate::cmd::Outcome;
use crate::config::{Backend, Config};
use crate::exit::{Exit, JevifyError};
use crate::source;

pub const WITHHELD_PATTERNS: [&str; 6] =
    [".*", "id_*", "*.pem", "*.key", "*credentials*", "*secret*"];

fn kinds() -> (Vec<serde_json::Value>, Option<String>) {
    let catalog = source::catalog(&source::Env::from_process(source::LISTER_TIMEOUT));
    let mut kinds: Vec<_> = catalog
        .kinds
        .iter()
        .map(|kind| serde_json::json!(kind))
        .collect();
    kinds.push(serde_json::json!({"name":"one", "origin":"coded", "family":"caller options", "list":[], "input":"@{one:a|b|c:question}; context on stdin or --context FILE"}));
    kinds.push(serde_json::json!({"name":"flag", "origin":"coded", "family":"caller options", "list":[], "input":"@{flag:--name:question}; yes keeps, no removes, unsure abstains"}));
    (kinds, catalog.error)
}

pub fn capabilities() -> Outcome {
    let (kinds, kinds_error) = kinds();
    let exit_codes: Vec<_> = Exit::ALL.iter().map(|(exit, meaning)| {
        serde_json::json!({"code":exit.code(), "name":exit, "meaning":meaning})
    }).collect();
    let error_kinds: Vec<_> = JevifyError::KINDS
        .iter()
        .map(|(kind, exit)| serde_json::json!({"kind":kind, "exit":exit.code()}))
        .collect();
    let data = serde_json::json!({
        "name":"jevify",
        "version":env!("CARGO_PKG_VERSION"),
        "summary":"Select existing records and command arguments by meaning; never generate them.",
        "global_flags":["--json (alias --robot)", "-t/--threshold <0..1>", "--model <id>", "--no-cache", "--verbose"],
        "commands":[
            {"name":"fill", "usage":"jevify fill [--dry-run] [-q] [-C DIR] [--candidates FILE] [--context FILE] [--field N | --key KEY] [-0 | --para] -- COMMAND ARGS...", "exit":[0,2,3,4,5,6], "data":"argv, markers, reason; exec mode has no envelope", "example":"jevify fill --dry-run -- git switch '@{branch:the auth refactor}'"},
            {"name":"pick", "usage":"jevify pick <intent...> [-n N] [--index | --files | --from KIND] [-0 | --para] [-C DIR]", "exit":[0,3], "data":"matches[{text,ordinal,p}], any, source; shortlist on abstention", "example":"jevify pick --from tool 'keep my mac awake'"},
            {"name":"why", "usage":"CMD 2>&1 | jevify why [-C N] [-n N] [--no-save]", "exit":[0,3], "data":"causes, any, considered, total, saved_input, complete", "example":"gh run view --log-failed | jevify why"},
            {"name":"filter", "usage":"LIST | jevify filter [-v] [-c] [--strict] [-0 | --para] [--files] [--no-save] <statement...>", "exit":[0,1,3], "data":"records, kept, total, unsure, complete, saved_input, excerpts_withheld"},
            {"name":"label", "usage":"LIST | jevify label a,b,c [-0 | --para] [--files]", "exit":[0,3], "data":"records, labelled, total, unsure, complete, excerpts_withheld"},
            {"name":"is", "usage":"jevify is <statement>... [--context FILE] [--band 0.15]", "exit":[0,1,3], "data":"p, verdict, truncated; statements for multiple questions"},
            {"name":"add", "usage":"jevify add [--dry-run | --yes] <topic...>", "exit":[0,2,3,6,130], "data":"hunks[{file,header,p,staged}]"},
            {"name":"capabilities", "usage":"jevify capabilities --json", "exit":[0], "data":"commands, kinds, exit_codes, error_kinds, env, envelope"},
            {"name":"health", "usage":"jevify health --json", "exit":[0,4,5], "data":"backend, base_url, key, api, latency_ms, models"},
            {"name":"init", "usage":"jevify init agents", "exit":[0], "data":"script"}
        ],
        "common_exit":[2,4,5,6],
        "exit_codes":exit_codes,
        "error_kinds":error_kinds,
        "kinds":kinds,
        "kinds_error":kinds_error,
        "env":[
            {"name":"TYPESAFE_API_KEY", "meaning":"select TypeSafe and authenticate; never printed"},
            {"name":"TYPESAFE_API_KEY_FILE", "meaning":"key file read only when needed"},
            {"name":"JEVIFY_BACKEND", "meaning":"typesafe or classifier; default follows key presence"},
            {"name":"JEVIFY_BASE_URL", "meaning":"pinned backend host; localhost allowed for testing; no redirects"},
            {"name":"JEVIFY_MODEL", "meaning":"TypeSafe model; unsupported on classifier.dev"},
            {"name":"JEVIFY_THRESHOLD", "default":0.5},
            {"name":"JEVIFY_CONCURRENCY", "meaning":"maximum concurrent backend requests"},
            {"name":"JEVIFY_DEADLINE", "default":600},
            {"name":"JEVIFY_STATUS_FILE", "meaning":"fill writes ran, argv, reason and error before exec; read ran to distinguish resolution failure from child exit"},
            {"name":"JEVIFY_CACHE_DIR", "meaning":"answer cache and saved inputs; tool inventory cached here when explicitly set"},
            {"name":"JEVIFY_CONFIG_DIR", "meaning":"explicit directory for user kinds.jsonl; no platform configuration directory or cwd recipes"},
            {"name":"JEVIFY_NO_CACHE", "meaning":"disable answer cache; independent of input saving and tool inventory"},
            {"name":"JEVIFY_NO_SAVE", "meaning":"1 disables raw input saving by why and filter"},
            {"name":"JEVIFY_DECISION", "meaning":"round_one includes tournament windows and finalists in meta.decision"},
            {"name":"JEVIFY_INVENTORY_FILE", "meaning":"JSON array of {name,summary} replaces PATH inventory for the tool kind"}
        ],
        "envelope":{
            "fields":["ok","command","version","exit_code","data","meta","error"],
            "branch_on":"exit_code, which equals the process exit code; then read data",
            "ok":"true on successful completion, including no and abstention; false on errors",
            "error":"{kind,message,hint,example}",
            "meta":"{backend,model,elapsed_ms,requests,cache_hits,input_tokens,threshold,request_id,usage,telemetry,decision}"
        },
        "safety":[
            "stdout carries handles and records; --json emits one envelope on one line",
            "fill: literal argv[0], no shell, nothing runs on abstention or error; never eval a dry-run preview",
            "fill stdin has one role; use --candidates or --context to separate inputs",
            "authorize fill per command prefix and add staging; other verbs start no user command",
            "below threshold, NONE and ties abstain; an unsure flag runs nothing",
            "obvious secrets are redacted before requests; secret files and symlink files get no excerpt",
            "only why and filter save raw input, secrets included; --no-save disables saving",
            "incomplete evidence cannot authorize a whole-input verdict; check considered, total and unsure",
            "lister overflow, failure and deadline are errors, never partial lists"
        ]
    });
    Outcome {
        exit: Exit::Ok,
        human: format!("{}\n", serde_json::to_string(&data).unwrap()).into_bytes(),
        exec: None,
        data,
    }
}

pub async fn health(ctx: &Config) -> Result<Outcome, JevifyError> {
    let base = crate::config::base_url(ctx.backend, Some(&ctx.base_url))?;
    let (path, key) = match ctx.backend {
        Backend::Typesafe => ("/v1/models", Some(ctx.api_key()?)),
        Backend::Classifier => ("/v1/health", None),
    };
    let mut req = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| JevifyError::Unavailable(e.to_string()))?
        .get(format!("{base}{path}"))
        .timeout(std::time::Duration::from_secs(5));
    if let Some(k) = &key {
        req = req.bearer_auth(k);
    }
    let start = std::time::Instant::now();
    let mut attempt = ctx.stats.start(crate::jev::client::AttemptKind::Health);
    let r = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            attempt.finish(false);
            return Err(JevifyError::Unavailable(e.to_string()));
        }
    };
    let ms = start.elapsed().as_millis();
    let backend = ctx.backend.as_str();
    match r.status().as_u16() {
        200 => {
            let bytes = match r.bytes().await {
                Ok(bytes) => bytes,
                Err(e) => {
                    attempt.finish(false);
                    return Err(JevifyError::Unavailable(e.to_string()));
                }
            };
            attempt.finish(true);
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
            let key_state = if key.is_some() {
                "present"
            } else {
                "not needed"
            };
            Ok(Outcome {
                exit: Exit::Ok,
                human: format!("ok: {backend} reachable in {ms} ms (key {key_state})\n")
                    .into_bytes(),
                exec: None,
                data: serde_json::json!({"backend":backend,"base_url":ctx.base_url,"key":key_state,"api":"reachable","latency_ms":ms,"models":body["models"]}),
            })
        }
        401 | 403 => {
            attempt.finish(false);
            Err(JevifyError::BadKey(r.status().as_u16()))
        }
        s => {
            attempt.finish(false);
            Err(JevifyError::Unavailable(format!("HTTP {s}")))
        }
    }
}

pub fn init(_shell: Shell) -> Outcome {
    let catalog = capabilities().data;
    let kinds = catalog["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|kind| kind["name"].as_str().unwrap())
        .collect::<Vec<_>>()
        .join(", ");
    let block = format!(
        "# jevify\nSelect existing handles and records by meaning when literal search cannot answer.\n\
         why: CMD 2>&1 | jevify why --json; inspect the cause, considered and total.\n\
         fill: jevify fill --dry-run -- git switch '@{{branch:the auth refactor}}'; omit --dry-run to execute.\n\
         pick: jevify pick --from tool 'keep my mac awake'; prints only the handle.\n\
         filter: LIST | jevify filter 'reports a failed assertion'; --strict drops unsure records.\n\
         label: LIST | jevify label bug,feature,question; prints LABEL<TAB>RECORD, ? when unsure.\n\
         is: jevify is 'asks for a refund' --context mail.txt; 0 yes, 1 no, 3 unsure.\n\
         add: jevify add --dry-run 'the token expiry fix'; --yes stages with caller authorization.\n\
         Kinds for fill markers and pick --from: {kinds}; one and flag are fill-only.\n\
         Quote whole markers. No shell is used; never eval the preview. An unsure flag runs nothing.\n\
         fill stdin has one role; separate --candidates and --context when needed.\n\
         Authorize fill per command prefix. JEVIFY_STATUS_FILE records ran before exec; after exec the command owns its exit code.\n\
         --json (alias --robot): one envelope on one line. Branch on exit_code, then data; error.kind identifies errors.\n\
         Exit: 0 ok, 1 no, 2 usage, 3 abstain, 4 unavailable, 5 auth, 6 input, 130 declined.\n\
         Abstention candidates are not answers; check exit before using stdout.\n\
         why and filter save raw input, secrets included; --no-save disables saving.\n\
         health: jevify health --json. capabilities: jevify capabilities --json lists commands, kinds and flags.\n\
         init: jevify init agents prints these instructions.\n"
    );
    Outcome {
        exit: Exit::Ok,
        data: serde_json::json!({"script":block}),
        human: block.into_bytes(),
        exec: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_list_every_kind_of_the_registry_with_its_argv_and_order() {
        let d = capabilities().data;
        let entries = d["kinds"].as_array().unwrap();
        let names: Vec<&str> = entries
            .iter()
            .map(|k| k["name"].as_str().unwrap())
            .collect();
        assert_eq!(names[0], "-");
        assert_eq!(&names[names.len() - 2..], ["one", "flag"]);
        let kind: std::collections::HashMap<_, _> = names.into_iter().zip(entries).collect();
        assert_eq!(kind["-"]["family"], "input records");
        assert!(kind["-"].get("ordered").is_none());
        assert_eq!(kind["branch"]["enrich"][10], "--");
        assert_eq!(kind["commit"]["ordered"], true);
        assert_eq!(kind["pod"]["origin"], "shipped");
        assert_eq!(
            kind["pod"]["list"],
            serde_json::json!(["kubectl", "get", "pods", "--no-headers"])
        );
        assert_eq!(kind["pod"]["ordered"], false);
        assert_eq!(kind["pr"]["ordered"], true);
        for name in ["-", "tool", "one", "flag"] {
            assert_eq!(kind[name]["list"], serde_json::json!([]), "{name}");
        }
    }

    #[test]
    fn init_agents_names_every_verb_every_listing_kind_and_the_capability_contract() {
        let block = String::from_utf8(init(Shell::Agents).human).unwrap();
        for verb in crate::VERBS {
            assert!(block.contains(verb));
        }
        assert!(block.contains("jevify capabilities --json"));
        assert!(block.lines().count() <= 25);
        let kinds = block.lines().find(|l| l.starts_with("Kinds")).unwrap();
        for kind in [
            "branch", "commit", "file", "dir", "tool", "pr", "ci-run", "pod",
        ] {
            assert!(kinds.contains(kind), "{kind}");
        }
    }
}
