#!/usr/bin/env python3
"""The data files the ergonomics tasks point at, written into a run's data/ directory.

    python3 scripts/ergonomics/fixtures.py OUT_DIR     # write them all, for inspection

Every file is deterministic: the same bytes on every run, so two runs of one task
differ only in what the agent did. The logs are long on purpose (more than 50
lines, the size at which `why` is the documented tool) and bury the one line
that matters among lines that look like it.
"""

import pathlib
import sys

TICKETS = """\
#101 The export to CSV writes the header twice when I pass --export-csv with a parameter scan.
#102 Could you add a way to run each benchmark in a random order? That would help with thermal drift.
#103 I was charged for the annual plan but I only wanted the monthly one; please send the difference back.
#104 How do I pass environment variables to the command being timed?
#105 The progress bar flickers and leaves garbage on my terminal in tmux.
#106 Please return my payment, the license key never arrived.
#107 It would be great to have a histogram in the terminal output itself.
#108 Is there a way to compare two git revisions of my tool automatically?
#109 Running with --shell=none crashes with a panic about an empty command.
#110 I'd like my money back for the duplicate order placed on Tuesday.
#111 Support for exporting results as a LaTeX table would be really useful.
#112 What does the 'user' time column actually measure?
"""
# refund requests: 103, 106, 110; bugs: 101, 105, 109; features: 102, 107, 111; questions: 104, 108, 112

MAIL_CANCEL = """\
From: dana.r@example.org
Subject: account

Hi,

We moved the whole team over to a different tool last quarter and nobody has
logged in since. Please make sure we are not billed again when the plan renews
on the 1st, and close the workspace for good. The invoices can go to the same
address as before if there is a final one.

Thanks,
Dana
"""

MAIL_STAY = """\
From: omar@example.net
Subject: thank you

Hello,

I was very close to cancelling last month because the sync kept failing, but
the fix you shipped on Friday solved it completely. We are renewing for another
year and I have asked finance to add two more seats. Keep up the good work.

Best,
Omar
"""

EVENTS = """\
09:00:01 api-gateway p99 latency 180ms, all regions serving
09:05:12 checkout error rate 0.2%, within budget
09:11:40 search responses slower than usual, 1.8s median, still returning results
09:14:03 payments: every request failing with 503, no successful charge in 4 minutes
09:15:30 image-cdn cache hit ratio 97%
09:20:44 login: users in eu-west cannot sign in at all, 100% of attempts rejected
09:22:10 recommendations served from stale cache, some items out of date
09:30:00 nightly backup completed in 41 minutes
09:31:17 email queue backlog of 20k messages, delivery delayed about 15 minutes
09:40:02 whole site returns a blank page, load balancer has no healthy targets
09:45:55 metrics pipeline healthy, lag 3s
09:50:21 search back to normal latency
"""
# outage: 09:14, 09:20, 09:40 -> 3


def _lines(prefix, n, fmt):
    return [fmt.format(prefix=prefix, i=i) for i in range(n)]


def timeouts_log():
    out = []
    for i in range(44):
        out.append(f"[worker-{i % 5}] job {1000 + i} finished ok in {12 + (i * 7) % 90}ms")
        if i == 9:
            out.append("[worker-4] job 1009 gave up: upstream did not answer within the 30s limit")
        if i == 21:
            out.append("[worker-1] job 1021 aborted: context deadline exceeded while reading from db")
        if i == 30:
            out.append("[worker-0] job 1030 retried twice after a 502 and then succeeded")
        if i == 33:
            out.append("[worker-3] job 1033 failed: operation timed out after 45 seconds")
        if i == 38:
            out.append("[worker-2] job 1038 failed: invalid payload, missing field 'id'")
    return "\n".join(out) + "\n"
# timeouts: 1009, 1021, 1033 -> 3


def build_log():
    out = ["   Compiling proc-macro2 v1.0.86", "   Compiling unicode-ident v1.0.12"]
    crates = ["libc", "cfg-if", "memchr", "serde", "serde_json", "itoa", "ryu", "anyhow",
              "clap_lex", "clap_builder", "clap", "regex-syntax", "regex-automata", "regex",
              "once_cell", "log", "bitflags", "rustix", "tempfile", "indicatif", "console",
              "unicode-width", "number_prefix", "portable-atomic", "statistical", "rand",
              "rand_core", "getrandom", "ppv-lite86", "rand_chacha", "nix", "shell-words",
              "csv", "csv-core", "thiserror", "thiserror-impl", "colored", "lazy_static"]
    for c in crates:
        out.append(f"   Compiling {c} v0.{len(c)}.{len(c) * 3 % 10}")
    out += [
        "   Compiling tally v0.4.0 (/work/tally)",
        "warning: unused import: `std::fmt::Write`",
        " --> src/report.rs:3:5",
        "  |",
        "3 | use std::fmt::Write;",
        "  |     ^^^^^^^^^^^^^^^",
        "  |",
        "  = note: `#[warn(unused_imports)]` on by default",
        "",
        "warning: variable does not need to be mutable",
        "  --> src/main.rs:41:9",
        "   |",
        "41 |     let mut total = 0;",
        "   |         ----^^^^^",
        "   |         |",
        "   |         help: remove this `mut`",
        "",
        "error[E0382]: borrow of moved value: `entries`",
        "  --> src/cache.rs:88:22",
        "   |",
        "81 |     let entries = load_entries(&path)?;",
        "   |         ------- move occurs because `entries` has type `Vec<Entry>`, which does not implement the `Copy` trait",
        "...",
        "85 |     let index = build_index(entries);",
        "   |                             ------- value moved here",
        "...",
        "88 |     for entry in entries.iter() {",
        "   |                  ^^^^^^^ value borrowed here after move",
        "",
        "warning: unused variable: `width`",
        "   --> src/report.rs:120:9",
        "    |",
        "120 |     let width = term_width();",
        "    |         ^^^^^ help: if this is intentional, prefix it with an underscore: `_width`",
        "",
        "For more information about this error, try `rustc --explain E0382`.",
        "warning: `tally` (bin \"tally\") generated 3 warnings",
        "error: could not compile `tally` (bin \"tally\") due to 1 previous error; 3 warnings emitted",
    ]
    return "\n".join(out) + "\n"


def pytest_log():
    out = ["============================= test session starts ==============================",
           "platform linux -- Python 3.12.4, pytest-8.2.2, pluggy-1.5.0",
           "rootdir: /builds/shop", "configfile: pyproject.toml",
           "collected 0 items / 6 errors", "",
           "==================================== ERRORS ===================================="]
    mods = ["test_cart.py", "test_checkout.py", "test_config.py", "test_orders.py",
            "test_refunds.py", "test_users.py"]
    for k, m in enumerate(mods):
        out += [f"_____________________ ERROR collecting tests/{m} _____________________",
                f"ImportError while importing test module '/builds/shop/tests/{m}'.",
                "Hint: make sure your test modules/packages have valid Python names.",
                "Traceback:",
                "/usr/lib/python3.12/importlib/__init__.py:90: in import_module",
                "    return _bootstrap._gcd_import(name[level:], package, level)",
                f"tests/{m}:3: in <module>",
                "    from shop.app import create_app",
                "shop/app.py:5: in <module>",
                "    from shop.settings import load_settings",
                "shop/settings.py:2: in <module>",
                "    import yaml" if k == 0 else "    from shop._compat import yaml_loader",
                "E   ModuleNotFoundError: No module named 'yaml'" if k == 0 else
                "E   ImportError: cannot import name 'yaml_loader' from partially initialized module 'shop._compat'"]
    out += ["=========================== short test summary info ============================"]
    out += [f"ERROR tests/{m}" for m in mods]
    out += ["!!!!!!!!!!!!!!!!!!! Interrupted: 6 errors during collection !!!!!!!!!!!!!!!!!!!!",
            "============================== 6 errors in 0.41s ==============================="]
    return "\n".join(out) + "\n"


def ci_lint_log():
    out = ["Run actions/checkout@v4", "Syncing repository: acme/storefront",
           "Run actions/setup-node@v4", "Found in cache @ /opt/hostedtoolcache/node/20.15.0/x64",
           "Run npm ci"]
    for i in range(30):
        out.append(f"npm WARN deprecated pkg-{i}@1.{i}.0: this version is no longer supported, please upgrade")
    out += ["added 1164 packages, and audited 1165 packages in 21s",
            "found 0 vulnerabilities",
            "Run npm run build",
            "> storefront@2.3.0 build", "> vite build",
            "vite v5.3.1 building for production...",
            "transforming (412) src/components/Cart.jsx",
            "✓ 530 modules transformed.",
            "dist/index.html                  0.46 kB",
            "dist/assets/index-4f1c.css      18.20 kB",
            "dist/assets/index-9a2b.js      212.77 kB",
            "✓ built in 4.12s",
            "Run npm run lint",
            "> storefront@2.3.0 lint", "> eslint src --max-warnings=0", "",
            "/home/runner/work/storefront/src/components/Cart.jsx",
            "  12:7  warning  Unexpected console statement  no-console",
            "  40:3  warning  React Hook useEffect has a missing dependency: 'items'  react-hooks/exhaustive-deps",
            "",
            "/home/runner/work/storefront/src/api/client.js",
            "  4:10  error  'retryDelay' is defined but never used  no-unused-vars",
            "",
            "/home/runner/work/storefront/src/pages/Checkout.jsx",
            "  88:5  warning  Unexpected console statement  no-console",
            "",
            "✖ 4 problems (1 error, 3 warnings)",
            "",
            "Error: Process completed with exit code 1.",
            "Post job cleanup.",
            "Cleaning up orphan processes"]
    return "\n".join(out) + "\n"


def gotest_log():
    out = ["go: downloading github.com/stretchr/testify v1.9.0"]
    tests = ["TestParseConfig", "TestParseConfigDefaults", "TestRouterNotFound", "TestRouterMethods",
             "TestAuthMiddleware", "TestAuthMiddlewareExpired", "TestRateLimit", "TestRateLimitBurst",
             "TestListOrders", "TestCreateOrder", "TestCreateOrderValidation", "TestHealthz"]
    for t in tests:
        out += [f"=== RUN   {t}", f"--- PASS: {t} (0.00s)"]
    out += ["=== RUN   TestGetOrder",
            "=== RUN   TestGetOrder/existing",
            "=== RUN   TestGetOrder/missing",
            "--- FAIL: TestGetOrder (0.00s)",
            "    --- PASS: TestGetOrder/existing (0.00s)",
            "    --- FAIL: TestGetOrder/missing (0.00s)",
            "panic: runtime error: invalid memory address or nil pointer dereference [recovered]",
            "\tpanic: runtime error: invalid memory address or nil pointer dereference",
            "[signal SIGSEGV: segmentation violation code=0x1 addr=0x18 pc=0x6f2c1a]",
            "",
            "goroutine 34 [running]:",
            "testing.tRunner.func1.2({0x71c3a0, 0x9a4f10})",
            "\t/usr/local/go/src/testing/testing.go:1631 +0x24a",
            "testing.tRunner.func1()",
            "\t/usr/local/go/src/testing/testing.go:1634 +0x377",
            "panic({0x71c3a0?, 0x9a4f10?})",
            "\t/usr/local/go/src/runtime/panic.go:770 +0x132",
            "example.com/shop/internal/api.(*Handler).GetOrder(0xc000130000, {0x7c3e58, 0xc0001421c0}, 0xc000150360)",
            "\t/builds/shop/internal/api/handler.go:42 +0x9a",
            "example.com/shop/internal/api.TestGetOrder.func2(0xc000182820)",
            "\t/builds/shop/internal/api/handler_test.go:77 +0x1c5",
            "testing.tRunner(0xc000182820, 0xc000110f50)",
            "\t/usr/local/go/src/testing/testing.go:1689 +0xfb",
            "created by testing.(*T).Run in goroutine 33",
            "\t/usr/local/go/src/testing/testing.go:1742 +0x390",
            "exit status 2",
            "FAIL\texample.com/shop/internal/api\t0.019s",
            "ok  \texample.com/shop/internal/config\t0.004s",
            "ok  \texample.com/shop/internal/ratelimit\t0.006s",
            "FAIL"]
    return "\n".join(out) + "\n"


INBOX = {
    "scan_0042.txt": "Brown the onions slowly in butter for twenty minutes, add two cloves of garlic, "
                     "then the stock. Simmer, blend half, season. Serves four.\n",
    "note_7.txt": "Outbound: LIS -> AMS, 14 Oct, departs 06:40, seat 21C, booking ref QX7P2L. "
                  "Return 19 Oct 18:05. Hotel near Centraal, check-in after 15:00.\n",
    "doc_3.txt": "Amount due: 1,240.00 EUR for consulting services, September. Payment within "
                 "30 days to the account below. Reference INV-2026-0931.\n",
    "file_11.txt": "Follow-up after the blood panel: vitamin D low, repeat in three months. "
                   "Continue the current dose; next appointment with Dr. Silva on 2 Dec.\n",
    "untitled.txt": "Mix flour, sugar and cold butter until crumbly, press into the tin, bake "
                    "at 180C for 15 minutes before adding the filling.\n",
    "img_2291.txt": "Train Paris -> Lyon, coach 7, seat 64, departs 09:12 from Gare de Lyon. "
                    "Keep the QR code for inspection.\n",
}
FILING = {"invoices": "Invoices and bills to pay.\n", "recipes": "Cooking recipes.\n",
          "travel": "Tickets, itineraries and bookings.\n", "medical": "Health records and appointments.\n"}


def write(out):
    out = pathlib.Path(out)
    out.mkdir(parents=True, exist_ok=True)
    files = {"tickets.txt": TICKETS, "mail_cancel.txt": MAIL_CANCEL, "mail_stay.txt": MAIL_STAY,
             "events.log": EVENTS, "jobs.log": timeouts_log(), "build.log": build_log(),
             "pytest.log": pytest_log(), "ci.log": ci_lint_log(), "gotest.log": gotest_log()}
    for name, text in files.items():
        (out / name).write_text(text)
    for name, text in INBOX.items():
        (out / "inbox").mkdir(exist_ok=True)
        (out / "inbox" / name).write_text(text)
    for folder, text in FILING.items():
        (out / "filing" / folder).mkdir(parents=True, exist_ok=True)
        (out / "filing" / folder / "ABOUT.txt").write_text(text)
    return out


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: fixtures.py OUT_DIR")
    print(write(sys.argv[1]))
