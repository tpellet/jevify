use crate::cli::Shell;
use crate::cmd::Outcome;
use crate::config::{Backend, Config, KEY_URL};
use crate::exit::{Exit, JevifyError};
use crate::jev::client::Client;
use crate::jev::{Question, Questions};
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
            {"name":"health", "usage":"jevify health --json", "exit":[0,4,5], "data":"backend, base_url, key, key_url, api, latency_ms, model"},
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
    // A cached answer cannot tell us whether the account can still classify.
    let probe = Config {
        backend: ctx.backend,
        key: ctx.key.clone(),
        key_file: ctx.key_file.clone(),
        base_url: ctx.base_url.clone(),
        model: ctx.model.clone(),
        threshold: ctx.threshold,
        concurrency: ctx.concurrency,
        cache_dir: None,
        stats: ctx.stats.clone(),
    };
    let start = std::time::Instant::now();
    let client = Client::new(&probe)?;
    let questions = Questions::from([(
        "health".into(),
        Question::noul("Does the text greet someone?"),
    )]);
    let answer = client
        .ask(&serde_json::json!("Hello there"), &questions)
        .await?;
    answer.noul("health")?;
    let ms = start.elapsed().as_millis();
    let backend = ctx.backend.as_str();
    let key_state = match ctx.backend {
        Backend::Typesafe => "present",
        Backend::Classifier => "not needed",
    };
    let key_note = match ctx.backend {
        Backend::Typesafe => String::new(),
        Backend::Classifier => format!("; a key from {KEY_URL} lifts the free budget"),
    };
    Ok(Outcome {
        exit: Exit::Ok,
        human: format!(
            "ok: {backend} answered with {} in {ms} ms (key {key_state}{key_note})\n",
            answer.model
        )
        .into_bytes(),
        exec: None,
        data: serde_json::json!({"backend":backend,"base_url":ctx.base_url,"key":key_state,"key_url":KEY_URL,"api":"answered","latency_ms":ms,"model":answer.model}),
    })
}

pub fn init(_shell: Shell) -> Outcome {
    let block = "# jevify\n\
         Use when you can describe a value but cannot name it, or a failure log is too long to read.\n\
         cargo test 2>&1 | jevify why\n\
         jevify fill --dry-run -- git show '@{commit:what it did}'\n\
         jevify pick --from commit 'what it did'\n\
         --from also accepts branch, file, pr, ci-run; stdout is the handle.\n\
         cargo test -- --list 2>/dev/null | sed -n 's/: test$//p' | jevify fill -- cargo test '@{-:what the test checks}' -- --exact\n\
         pytest --collect-only -q | sed -n '/::/p' | jevify fill -- pytest '@{-:what the test checks}'\n\
         cat records.txt | jevify filter 'reports a failed assertion'\n\
         cat records.txt | jevify label bug,feature,question\n\
         jevify is 'reports a failure' < build.log\n\
         Single-quote whole markers. Never eval a preview. Omit --dry-run only with authorization.\n\
         fill stdin has one role; separate --candidates and --context. An unsure flag runs nothing.\n\
         JEVIFY_STATUS_FILE records whether fill ran; after exec the child owns its exit code.\n\
         --json: one envelope; branch on exit_code, then data.\n\
         0 found/yes; 1 no; 2 usage: run error.example after checking authorization.\n\
         3 nothing fits: read data.shortlist if present; candidates are not answers.\n\
         4 unavailable: error.kind quota_exhausted means stop for today; 5 auth; 6 input.\n\
         jevify add --dry-run 'the auth fix'; --yes authorizes staging; 130 means a person declined.\n\
         why and filter save raw input; --no-save disables saving.\n\
         jevify health --json checks classification; jevify capabilities --json lists commands and kinds.\n".to_owned();
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
}
