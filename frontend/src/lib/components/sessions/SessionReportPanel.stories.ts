// @ts-nocheck — Storybook v10 types don't yet fully support Svelte 5 runes-mode components.
// Fixtures are one real prod session (ololo.dev/s/3WTCA2), trimmed to what the
// panel reads: the judges' verdicts, the criteria sheet and the report the
// Debrief wrote about them.
import type { Meta, StoryObj } from "@storybook/sveltekit";
import SessionReportPanel from "$lib/components/sessions/SessionReportPanel.svelte";

const tasks = [
  {
    task_id: "19d84f07-fb4f-407c-998f-b5880078895f",
    ordinal: 0,
    title: "Carry the earlier parts forward",
    content:
      "Setup only \u2014 do NOT build crypto yet, that is the next task.\n\nThis part continues the product you already built \u2014 the ledger,\nreceipts, and money held in several currencies.\n\nSame repository, same documented commands. If you are starting in an\nempty folder, your earlier work is fetched for you before you begin.\n\nBefore writing anything new, check that what you carried in still stands:\n\n- AGENTS.md still declares `stack:`, `run:` and `test:`;\n- the suite the `test:` line names is green;\n- the parts already delivered are still delivered.\n\nAnd check the product itself, not only its suite. These must still be true\nof what you carried in:\n\n- the March book still balances at 3223.70, and receipts still become\n  drafts a person confirms\n- a 120.00 USD entry on 2026-03-14 still keeps its 120.00 USD and\n  reports as 112.80 EUR\n- a date without a rate is still shown as missing rather than guessed\n\nFix whatever the hand-over broke before you start this part \u2014 a judge reads\nthe product on this rung, and everything the earlier parts earned is judged\nagain at the end of it, so a regression carried in quietly costs twice.",
    adapted_content:
      "cd {baseDir}\nsed -n 's/^run: *//p' AGENTS.md README.md 2>/dev/null | head -1 | tr -d '\\r'",
    tags: ["setup", "environment"],
    result: {
      status: "completed",
      submitted_answer: "",
      correct_answer: null,
      score_delta: 10,
      evaluated_at: "2026-08-22T20:18:17.890096Z",
    },
    scheduler_state: null,
    total_points: 20,
    bonus_points: 10,
  },
  {
    task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
    ordinal: 1,
    title: "Track crypto beside the cash",
    content:
      "Coins sit next to cash now: bought in fractions, priced by a market that\nmoves while nobody is looking, and worth being honest about. You decide when\nit is done; a judge panel scores the result.\n\nThe prices the product knows, in the home currency EUR:\n\n  | date       | asset | price    |\n  | 2026-03-02 | BTC   | 61240.00 |\n  | 2026-03-02 | ETH   |  3180.00 |\n  | 2026-03-02 | USDC  |     0.92 |\n  | 2026-03-20 | BTC   | 58900.00 |\n  | 2026-03-20 | ETH   |  3415.00 |\n  | 2026-03-20 | USDC  |     0.94 |\n\nScenarios:\n\n  Scenario: Buying a fraction\n    When 0.01750000 BTC is bought on 2026-03-02\n    Then the holding reads 0.01750000 BTC\n    And it cost 1071.70 EUR, which the product remembers\n\n  Scenario: Eight decimals survive\n    When 0.00000001 BTC is added to a holding\n    Then the holding says so, to the last decimal\n    And no amount is ever rounded on its way in or out\n\n  Scenario: What it is worth today\n    Given the holding of 0.01750000 BTC bought on 2026-03-02\n    When it is valued on 2026-03-20\n    Then it is worth 1030.75 EUR\n    And the unrealised result reads -40.95\n\n  Scenario: Selling settles part of it\n    When 0.00750000 BTC is sold on 2026-03-20\n    Then 0.01000000 BTC is left\n    And the realised result of -17.55 is recorded\n\n  Scenario: Cash and coins in one number\n    Then the money the person has is one figure in the home currency\n    And it includes accounts and holdings alike\n\n  Scenario: A price that is missing\n    When today's price of an asset is unknown\n    Then the holding is shown at the last price the product knows, with its date\n    And never as zero\n\nEverything beyond the scenarios is yours: where prices come from, how a\nwallet is added, how gains are presented.\n\nWhen you are done, write .ololo/money-tracker-crypto-done.md with a short\ndescription of the implemented solution (at least 10 words).",
    adapted_content:
      'cd {baseDir}\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
    tags: ["money", "product"],
    result: {
      status: "pending",
      submitted_answer: "",
      correct_answer: null,
      score_delta: 0,
      evaluated_at: "2026-08-22T20:54:29.605363Z",
    },
    scheduler_state: {
      state: "idle",
      activated_at: null,
      deadline_at: "2026-08-22T20:59:40.714322Z",
    },
    total_points: 108,
    bonus_points: 0,
  },
];

const probesByTask = new Map(
  Object.entries({
    "19d84f07-fb4f-407c-998f-b5880078895f": [
      {
        id: "f6e90fcf-1f2f-4fcc-b86f-eb597c52b772",
        task_id: "19d84f07-fb4f-407c-998f-b5880078895f",
        task_title: "Carry the earlier parts forward",
        task_ordinal: 0,
        adapted_test_id: "b9221b54-6fab-4899-b200-7d6d4df68129",
        test_ordinal: 0,
        label: "Declare how to run the project",
        description:
          "AGENTS.md (or README.md) must carry the `run:` line with the command that runs the product. It is captured into this session's memory and read by the judges. AGENTS.md wins when both declare one.",
        test_command:
          "cd {baseDir}\nsed -n 's/^run: *//p' AGENTS.md README.md 2>/dev/null | head -1 | tr -d '\\r'",
        attempt: 1,
        rendered_command:
          "cd .\nsed -n 's/^run: *//p' AGENTS.md README.md 2>/dev/null | head -1 | tr -d '\\r'",
        fixture_values: '{"baseDir":"."}',
        expected_answer: "result.trim().length > 0",
        state: "resolved",
        outcome: "pass",
        actual: "npm start",
        expected: "result.trim().length > 0",
        exit_code: 0,
        duration_ms: 179,
        dispatched_at: "2026-08-22T20:17:59.977300Z",
        deadline_at: "2026-08-22T20:18:59.977300Z",
        resolved_at: "2026-08-22T20:18:11.545285Z",
        point_delta: 0,
        updated_at: "2026-08-22T20:18:11.545285Z",
        result: {
          status: "passed",
          expected: "result.trim().length > 0",
          actual: "npm start",
        },
      },
      {
        id: "e5db5fe0-0dcc-4083-a440-f6fd7f608856",
        task_id: "19d84f07-fb4f-407c-998f-b5880078895f",
        task_title: "Carry the earlier parts forward",
        task_ordinal: 0,
        adapted_test_id: "e381377a-26e8-4c7c-8203-74fa7efb7d6a",
        test_ordinal: 1,
        label: "Declare how to test the project",
        description:
          "AGENTS.md (or README.md) must carry the `test:` line with the command that runs the suite. The check below runs exactly what it says.",
        test_command:
          "cd {baseDir}\nsed -n 's/^test: *//p' AGENTS.md README.md 2>/dev/null | head -1 | tr -d '\\r'",
        attempt: 1,
        rendered_command:
          "cd .\nsed -n 's/^test: *//p' AGENTS.md README.md 2>/dev/null | head -1 | tr -d '\\r'",
        fixture_values: '{"baseDir":"."}',
        expected_answer: "result.trim().length > 0",
        state: "resolved",
        outcome: "pass",
        actual: "npm test",
        expected: "result.trim().length > 0",
        exit_code: 0,
        duration_ms: 12,
        dispatched_at: "2026-08-22T20:18:12.563483Z",
        deadline_at: "2026-08-22T20:19:12.563483Z",
        resolved_at: "2026-08-22T20:18:12.628836Z",
        point_delta: 0,
        updated_at: "2026-08-22T20:18:12.628836Z",
        result: {
          status: "passed",
          expected: "result.trim().length > 0",
          actual: "npm test",
        },
      },
      {
        id: "535d1e6e-dab0-4be2-ad1c-e6a53e4416cc",
        task_id: "19d84f07-fb4f-407c-998f-b5880078895f",
        task_title: "Carry the earlier parts forward",
        task_ordinal: 0,
        adapted_test_id: "e86dd82f-d826-471b-9a75-aed4b68a21ab",
        test_ordinal: 2,
        label: "What you carried in still passes its own tests",
        description:
          "The suite your `test:` line names, run against the code this session started from. A hand-over that arrives broken is worth knowing about in the first minute, not at the end.",
        test_command:
          'cd {baseDir}\nT={memory.test}\nmkdir -p .ololo/tmp\nif $T >.ololo/tmp/smoke.log 2>&1; then\n  echo "suite: green"\nelse\n  echo "not-green: \'$T\' failed on the code you carried in - fix it before building crypto"\n  tail -5 .ololo/tmp/smoke.log 2>/dev/null\nfi',
        attempt: 1,
        rendered_command:
          "cd .\nT='npm test'\nmkdir -p .ololo/tmp\nif $T >.ololo/tmp/smoke.log 2>&1; then\n  echo \"suite: green\"\nelse\n  echo \"not-green: '$T' failed on the code you carried in - fix it before building crypto\"\n  tail -5 .ololo/tmp/smoke.log 2>/dev/null\nfi",
        fixture_values: '{"baseDir":"."}',
        expected_answer: 'result.includes("suite: green")',
        state: "resolved",
        outcome: "pass",
        actual: "suite: green",
        expected: 'result.includes("suite: green")',
        exit_code: 0,
        duration_ms: 2036,
        dispatched_at: "2026-08-22T20:18:13.648952Z",
        deadline_at: "2026-08-22T20:19:13.648952Z",
        resolved_at: "2026-08-22T20:18:15.754003Z",
        point_delta: 0,
        updated_at: "2026-08-22T20:18:15.754003Z",
        result: {
          status: "passed",
          expected: 'result.includes("suite: green")',
          actual: "suite: green",
        },
      },
      {
        id: "914456e1-ab4b-4ae4-bc75-5e886567016f",
        task_id: "19d84f07-fb4f-407c-998f-b5880078895f",
        task_title: "Carry the earlier parts forward",
        task_ordinal: 0,
        adapted_test_id: "7bf42170-6763-48c5-bbec-2c9141cc9b7f",
        test_ordinal: 3,
        label: "Everything the earlier parts delivered is still here",
        description:
          "The campaign is one product. The parts already finished left their notes in the repository, and the stack is the one declared in part one.",
        test_command:
          'cd {baseDir}\nfor f in money-tracker-ledger-done.md money-tracker-receipts-done.md money-tracker-currencies-done.md; do\n  test -f ".ololo/$f" || { echo "missing: .ololo/$f - that part is meant to be in this codebase; start from your earlier work rather than an empty folder"; exit 0; }\ndone\ngrep -Eqs "^stack: *\\S" AGENTS.md README.md || { echo "missing: AGENTS.md needs its \'stack:\' line - one campaign, one declared stack"; exit 0; }\necho "carried: the earlier parts are here"',
        attempt: 1,
        rendered_command:
          'cd .\nfor f in money-tracker-ledger-done.md money-tracker-receipts-done.md money-tracker-currencies-done.md; do\n  test -f ".ololo/$f" || { echo "missing: .ololo/$f - that part is meant to be in this codebase; start from your earlier work rather than an empty folder"; exit 0; }\ndone\ngrep -Eqs "^stack: *\\S" AGENTS.md README.md || { echo "missing: AGENTS.md needs its \'stack:\' line - one campaign, one declared stack"; exit 0; }\necho "carried: the earlier parts are here"',
        fixture_values: '{"baseDir":"."}',
        expected_answer: 'result.includes("carried:")',
        state: "resolved",
        outcome: "pass",
        actual: "carried: the earlier parts are here",
        expected: 'result.includes("carried:")',
        exit_code: 0,
        duration_ms: 16,
        dispatched_at: "2026-08-22T20:18:16.780126Z",
        deadline_at: "2026-08-22T20:19:16.780126Z",
        resolved_at: "2026-08-22T20:18:16.852252Z",
        point_delta: 0,
        updated_at: "2026-08-22T20:18:16.852252Z",
        result: {
          status: "passed",
          expected: 'result.includes("carried:")',
          actual: "carried: the earlier parts are here",
        },
      },
    ],
    "c5c47937-ce66-4c19-b2eb-030da0257b8d": [
      {
        id: "591355dd-96bb-475a-93c7-2240b1e504d1",
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        task_title: "Track crypto beside the cash",
        task_ordinal: 1,
        adapted_test_id: "8b8b8389-cac5-4495-9c79-38f1d78a583f",
        test_ordinal: 0,
        label: "Definition of done",
        test_command:
          'cd {baseDir}\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        attempt: 1,
        rendered_command:
          'cd .\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        fixture_values: '{"baseDir":"."}',
        expected_answer: 'result.includes("done-note: present")',
        state: "resolved",
        outcome: "error",
        actual:
          "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        expected: 'result.includes("done-note: present")',
        exit_code: 0,
        duration_ms: 25,
        dispatched_at: "2026-08-22T20:18:17.907128Z",
        deadline_at: "2026-08-22T20:19:47.907128Z",
        resolved_at: "2026-08-22T20:18:18.075326Z",
        point_delta: 0,
        updated_at: "2026-08-22T20:18:18.075326Z",
        result: {
          status: "failed",
          expected: 'result.includes("done-note: present")',
          actual:
            "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        },
      },
      {
        id: "41fc3b02-c61a-4975-9c60-76d646c808bd",
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        task_title: "Track crypto beside the cash",
        task_ordinal: 1,
        adapted_test_id: "8b8b8389-cac5-4495-9c79-38f1d78a583f",
        test_ordinal: 0,
        label: "Definition of done",
        test_command:
          'cd {baseDir}\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        attempt: 1,
        rendered_command:
          'cd .\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        fixture_values: '{"baseDir":"."}',
        expected_answer: 'result.includes("done-note: present")',
        state: "resolved",
        outcome: "error",
        actual:
          "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        expected: 'result.includes("done-note: present")',
        exit_code: 0,
        duration_ms: 25,
        dispatched_at: "2026-08-22T20:18:58.109492Z",
        deadline_at: "2026-08-22T20:20:28.109492Z",
        resolved_at: "2026-08-22T20:18:58.194776Z",
        point_delta: 0,
        updated_at: "2026-08-22T20:18:58.194776Z",
        result: {
          status: "failed",
          expected: 'result.includes("done-note: present")',
          actual:
            "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        },
      },
      {
        id: "75c1b127-9e14-4a25-8fb4-7f4c11eb2711",
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        task_title: "Track crypto beside the cash",
        task_ordinal: 1,
        adapted_test_id: "8b8b8389-cac5-4495-9c79-38f1d78a583f",
        test_ordinal: 0,
        label: "Definition of done",
        test_command:
          'cd {baseDir}\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        attempt: 1,
        rendered_command:
          'cd .\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        fixture_values: '{"baseDir":"."}',
        expected_answer: 'result.includes("done-note: present")',
        state: "resolved",
        outcome: "error",
        actual:
          "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        expected: 'result.includes("done-note: present")',
        exit_code: 0,
        duration_ms: 25,
        dispatched_at: "2026-08-22T20:19:58.220949Z",
        deadline_at: "2026-08-22T20:21:28.220949Z",
        resolved_at: "2026-08-22T20:19:58.302758Z",
        point_delta: 0,
        updated_at: "2026-08-22T20:19:58.302758Z",
        result: {
          status: "failed",
          expected: 'result.includes("done-note: present")',
          actual:
            "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        },
      },
      {
        id: "acb4397e-b385-4f22-a706-7f30ada8ce6e",
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        task_title: "Track crypto beside the cash",
        task_ordinal: 1,
        adapted_test_id: "8b8b8389-cac5-4495-9c79-38f1d78a583f",
        test_ordinal: 0,
        label: "Definition of done",
        test_command:
          'cd {baseDir}\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        attempt: 1,
        rendered_command:
          'cd .\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        fixture_values: '{"baseDir":"."}',
        expected_answer: 'result.includes("done-note: present")',
        state: "resolved",
        outcome: "error",
        actual:
          "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        expected: 'result.includes("done-note: present")',
        exit_code: 0,
        duration_ms: 31,
        dispatched_at: "2026-08-22T20:21:18.330874Z",
        deadline_at: "2026-08-22T20:22:48.330874Z",
        resolved_at: "2026-08-22T20:21:18.478097Z",
        point_delta: 0,
        updated_at: "2026-08-22T20:21:18.478097Z",
        result: {
          status: "failed",
          expected: 'result.includes("done-note: present")',
          actual:
            "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        },
      },
      {
        id: "32efa102-0a1d-4d6f-8ff6-f4f1ab40cceb",
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        task_title: "Track crypto beside the cash",
        task_ordinal: 1,
        adapted_test_id: "8b8b8389-cac5-4495-9c79-38f1d78a583f",
        test_ordinal: 0,
        label: "Definition of done",
        test_command:
          'cd {baseDir}\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        attempt: 1,
        rendered_command:
          'cd .\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        fixture_values: '{"baseDir":"."}',
        expected_answer: 'result.includes("done-note: present")',
        state: "resolved",
        outcome: "error",
        actual:
          "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        expected: 'result.includes("done-note: present")',
        exit_code: 0,
        duration_ms: 23,
        dispatched_at: "2026-08-22T20:22:58.508210Z",
        deadline_at: "2026-08-22T20:24:28.508210Z",
        resolved_at: "2026-08-22T20:22:58.608879Z",
        point_delta: 0,
        updated_at: "2026-08-22T20:22:58.608879Z",
        result: {
          status: "failed",
          expected: 'result.includes("done-note: present")',
          actual:
            "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        },
      },
      {
        id: "d30a0127-5748-495b-a82f-3c66891d4711",
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        task_title: "Track crypto beside the cash",
        task_ordinal: 1,
        adapted_test_id: "8b8b8389-cac5-4495-9c79-38f1d78a583f",
        test_ordinal: 0,
        label: "Definition of done",
        test_command:
          'cd {baseDir}\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        attempt: 1,
        rendered_command:
          'cd .\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        fixture_values: '{"baseDir":"."}',
        expected_answer: 'result.includes("done-note: present")',
        state: "resolved",
        outcome: "error",
        actual:
          "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        expected: 'result.includes("done-note: present")',
        exit_code: 0,
        duration_ms: 40,
        dispatched_at: "2026-08-22T20:24:58.636124Z",
        deadline_at: "2026-08-22T20:26:28.636124Z",
        resolved_at: "2026-08-22T20:24:58.736237Z",
        point_delta: 0,
        updated_at: "2026-08-22T20:24:58.736237Z",
        result: {
          status: "failed",
          expected: 'result.includes("done-note: present")',
          actual:
            "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        },
      },
      {
        id: "d87d7862-b0df-45a7-b5b1-91b2ac0c229b",
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        task_title: "Track crypto beside the cash",
        task_ordinal: 1,
        adapted_test_id: "8b8b8389-cac5-4495-9c79-38f1d78a583f",
        test_ordinal: 0,
        label: "Definition of done",
        test_command:
          'cd {baseDir}\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        attempt: 1,
        rendered_command:
          'cd .\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        fixture_values: '{"baseDir":"."}',
        expected_answer: 'result.includes("done-note: present")',
        state: "resolved",
        outcome: "error",
        actual:
          "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        expected: 'result.includes("done-note: present")',
        exit_code: 0,
        duration_ms: 32,
        dispatched_at: "2026-08-22T20:26:58.753707Z",
        deadline_at: "2026-08-22T20:28:28.753707Z",
        resolved_at: "2026-08-22T20:26:58.842327Z",
        point_delta: 0,
        updated_at: "2026-08-22T20:26:58.842327Z",
        result: {
          status: "failed",
          expected: 'result.includes("done-note: present")',
          actual:
            "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        },
      },
      {
        id: "bf66397e-26de-406b-a577-326006221ce8",
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        task_title: "Track crypto beside the cash",
        task_ordinal: 1,
        adapted_test_id: "8b8b8389-cac5-4495-9c79-38f1d78a583f",
        test_ordinal: 0,
        label: "Definition of done",
        test_command:
          'cd {baseDir}\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        attempt: 1,
        rendered_command:
          'cd .\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        fixture_values: '{"baseDir":"."}',
        expected_answer: 'result.includes("done-note: present")',
        state: "resolved",
        outcome: "error",
        actual:
          "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        expected: 'result.includes("done-note: present")',
        exit_code: 0,
        duration_ms: 25,
        dispatched_at: "2026-08-22T20:28:58.866469Z",
        deadline_at: "2026-08-22T20:30:28.866469Z",
        resolved_at: "2026-08-22T20:28:59.001688Z",
        point_delta: 0,
        updated_at: "2026-08-22T20:28:59.001688Z",
        result: {
          status: "failed",
          expected: 'result.includes("done-note: present")',
          actual:
            "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        },
      },
      {
        id: "eba50ac1-75dc-4515-b166-4987eae69c30",
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        task_title: "Track crypto beside the cash",
        task_ordinal: 1,
        adapted_test_id: "8b8b8389-cac5-4495-9c79-38f1d78a583f",
        test_ordinal: 0,
        label: "Definition of done",
        test_command:
          'cd {baseDir}\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        attempt: 1,
        rendered_command:
          'cd .\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        fixture_values: '{"baseDir":"."}',
        expected_answer: 'result.includes("done-note: present")',
        state: "resolved",
        outcome: "error",
        actual:
          "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        expected: 'result.includes("done-note: present")',
        exit_code: 0,
        duration_ms: 19,
        dispatched_at: "2026-08-22T20:30:59.032127Z",
        deadline_at: "2026-08-22T20:32:29.032127Z",
        resolved_at: "2026-08-22T20:30:59.112605Z",
        point_delta: 0,
        updated_at: "2026-08-22T20:30:59.112605Z",
        result: {
          status: "failed",
          expected: 'result.includes("done-note: present")',
          actual:
            "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        },
      },
      {
        id: "1f7fef77-a391-4b88-8b17-1a89bbe0221e",
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        task_title: "Track crypto beside the cash",
        task_ordinal: 1,
        adapted_test_id: "8b8b8389-cac5-4495-9c79-38f1d78a583f",
        test_ordinal: 0,
        label: "Definition of done",
        test_command:
          'cd {baseDir}\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        attempt: 1,
        rendered_command:
          'cd .\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        fixture_values: '{"baseDir":"."}',
        expected_answer: 'result.includes("done-note: present")',
        state: "resolved",
        outcome: "error",
        actual:
          "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        expected: 'result.includes("done-note: present")',
        exit_code: 0,
        duration_ms: 23,
        dispatched_at: "2026-08-22T20:32:59.148084Z",
        deadline_at: "2026-08-22T20:34:29.148084Z",
        resolved_at: "2026-08-22T20:32:59.232759Z",
        point_delta: 0,
        updated_at: "2026-08-22T20:32:59.232759Z",
        result: {
          status: "failed",
          expected: 'result.includes("done-note: present")',
          actual:
            "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        },
      },
      {
        id: "e2b9a897-b401-4938-b6a7-282a65a34bcb",
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        task_title: "Track crypto beside the cash",
        task_ordinal: 1,
        adapted_test_id: "8b8b8389-cac5-4495-9c79-38f1d78a583f",
        test_ordinal: 0,
        label: "Definition of done",
        test_command:
          'cd {baseDir}\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        attempt: 1,
        rendered_command:
          'cd .\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        fixture_values: '{"baseDir":"."}',
        expected_answer: 'result.includes("done-note: present")',
        state: "resolved",
        outcome: "error",
        actual:
          "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        expected: 'result.includes("done-note: present")',
        exit_code: 0,
        duration_ms: 22,
        dispatched_at: "2026-08-22T20:34:59.265251Z",
        deadline_at: "2026-08-22T20:36:29.265251Z",
        resolved_at: "2026-08-22T20:34:59.345734Z",
        point_delta: 0,
        updated_at: "2026-08-22T20:34:59.345734Z",
        result: {
          status: "failed",
          expected: 'result.includes("done-note: present")',
          actual:
            "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        },
      },
      {
        id: "76731ea6-a814-4152-aa4b-7ca37f2d3d71",
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        task_title: "Track crypto beside the cash",
        task_ordinal: 1,
        adapted_test_id: "8b8b8389-cac5-4495-9c79-38f1d78a583f",
        test_ordinal: 0,
        label: "Definition of done",
        test_command:
          'cd {baseDir}\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        attempt: 1,
        rendered_command:
          'cd .\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        fixture_values: '{"baseDir":"."}',
        expected_answer: 'result.includes("done-note: present")',
        state: "resolved",
        outcome: "error",
        actual:
          "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        expected: 'result.includes("done-note: present")',
        exit_code: 0,
        duration_ms: 21,
        dispatched_at: "2026-08-22T20:36:59.364550Z",
        deadline_at: "2026-08-22T20:38:29.364550Z",
        resolved_at: "2026-08-22T20:36:59.441529Z",
        point_delta: 0,
        updated_at: "2026-08-22T20:36:59.441529Z",
        result: {
          status: "failed",
          expected: 'result.includes("done-note: present")',
          actual:
            "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal",
        },
      },
      {
        id: "0bf6c5ee-e48e-452a-8242-f4242ab767e6",
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        task_title: "Track crypto beside the cash",
        task_ordinal: 1,
        adapted_test_id: "8b8b8389-cac5-4495-9c79-38f1d78a583f",
        test_ordinal: 0,
        label: "Definition of done",
        test_command:
          'cd {baseDir}\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        attempt: 1,
        rendered_command:
          'cd .\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        fixture_values: '{"baseDir":"."}',
        expected_answer: 'result.includes("done-note: present")',
        state: "resolved",
        outcome: "no_response",
        actual: null,
        expected: 'result.includes("done-note: present")',
        exit_code: null,
        duration_ms: null,
        dispatched_at: "2026-08-22T20:38:59.462663Z",
        deadline_at: "2026-08-22T20:40:29.462663Z",
        resolved_at: null,
        point_delta: 0,
        updated_at: "2026-08-22T20:40:29.469732Z",
        result: {
          status: "no_response",
          expected: 'result.includes("done-note: present")',
          actual: null,
        },
      },
      {
        id: "13ff3de8-c804-4116-bbe6-aaa02176ccb5",
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        task_title: "Track crypto beside the cash",
        task_ordinal: 1,
        adapted_test_id: "8b8b8389-cac5-4495-9c79-38f1d78a583f",
        test_ordinal: 0,
        label: "Definition of done",
        test_command:
          'cd {baseDir}\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        attempt: 1,
        rendered_command:
          'cd .\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        fixture_values: '{"baseDir":"."}',
        expected_answer: 'result.includes("done-note: present")',
        state: "resolved",
        outcome: "no_response",
        actual: null,
        expected: 'result.includes("done-note: present")',
        exit_code: null,
        duration_ms: null,
        dispatched_at: "2026-08-22T20:42:29.495130Z",
        deadline_at: "2026-08-22T20:43:59.495130Z",
        resolved_at: null,
        point_delta: 0,
        updated_at: "2026-08-22T20:43:59.501941Z",
        result: {
          status: "no_response",
          expected: 'result.includes("done-note: present")',
          actual: null,
        },
      },
      {
        id: "8f5e826b-2253-47c4-832f-75c884124255",
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        task_title: "Track crypto beside the cash",
        task_ordinal: 1,
        adapted_test_id: "8b8b8389-cac5-4495-9c79-38f1d78a583f",
        test_ordinal: 0,
        label: "Definition of done",
        test_command:
          'cd {baseDir}\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        attempt: 1,
        rendered_command:
          'cd .\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        fixture_values: '{"baseDir":"."}',
        expected_answer: 'result.includes("done-note: present")',
        state: "resolved",
        outcome: "no_response",
        actual: null,
        expected: 'result.includes("done-note: present")',
        exit_code: null,
        duration_ms: null,
        dispatched_at: "2026-08-22T20:45:59.529271Z",
        deadline_at: "2026-08-22T20:47:29.529271Z",
        resolved_at: null,
        point_delta: 0,
        updated_at: "2026-08-22T20:47:29.534217Z",
        result: {
          status: "no_response",
          expected: 'result.includes("done-note: present")',
          actual: null,
        },
      },
      {
        id: "fef91050-2f4a-477e-b9de-ebd49cca71c3",
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        task_title: "Track crypto beside the cash",
        task_ordinal: 1,
        adapted_test_id: "8b8b8389-cac5-4495-9c79-38f1d78a583f",
        test_ordinal: 0,
        label: "Definition of done",
        test_command:
          'cd {baseDir}\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        attempt: 1,
        rendered_command:
          'cd .\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        fixture_values: '{"baseDir":"."}',
        expected_answer: 'result.includes("done-note: present")',
        state: "resolved",
        outcome: "no_response",
        actual: null,
        expected: 'result.includes("done-note: present")',
        exit_code: null,
        duration_ms: null,
        dispatched_at: "2026-08-22T20:49:29.559206Z",
        deadline_at: "2026-08-22T20:50:59.559206Z",
        resolved_at: null,
        point_delta: 0,
        updated_at: "2026-08-22T20:50:59.567168Z",
        result: {
          status: "no_response",
          expected: 'result.includes("done-note: present")',
          actual: null,
        },
      },
      {
        id: "48954c5d-3635-4177-af24-bee16d645c1a",
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        task_title: "Track crypto beside the cash",
        task_ordinal: 1,
        adapted_test_id: "8b8b8389-cac5-4495-9c79-38f1d78a583f",
        test_ordinal: 0,
        label: "Definition of done",
        test_command:
          'cd {baseDir}\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        attempt: 1,
        rendered_command:
          'cd .\ntest -f .ololo/money-tracker-crypto-done.md || { echo "not-done: .ololo/money-tracker-crypto-done.md is missing - write it when holdings, valuations and realised results are right to the last decimal"; exit 0; }\nwords=$(wc -w < .ololo/money-tracker-crypto-done.md | tr -d \' \')\n[ "$words" -ge 10 ] || { echo "not-done: .ololo/money-tracker-crypto-done.md needs a short description of what you built (at least 10 words)"; exit 0; }\ngrep -Eqs "^run: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'run:\' line with the command that runs the app"; exit 0; }\ngrep -Eqs "^test: *\\S" AGENTS.md README.md || { echo "not-done: AGENTS.md needs a \'test:\' line with the command that runs the tests"; exit 0; }\necho "done-note: present"',
        fixture_values: '{"baseDir":"."}',
        expected_answer: 'result.includes("done-note: present")',
        state: "resolved",
        outcome: "no_response",
        actual: null,
        expected: 'result.includes("done-note: present")',
        exit_code: null,
        duration_ms: null,
        dispatched_at: "2026-08-22T20:52:59.593094Z",
        deadline_at: "2026-08-22T20:54:29.593094Z",
        resolved_at: null,
        point_delta: 0,
        updated_at: "2026-08-22T20:54:29.601153Z",
        result: {
          status: "no_response",
          expected: 'result.includes("done-note: present")',
          actual: null,
        },
      },
      {
        id: "ded12b8c-e8b6-480e-9ab8-48e2e4ced8ec",
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        task_title: "Track crypto beside the cash",
        task_ordinal: 1,
        adapted_test_id: "f7cd9cdc-947f-4a0a-8d3e-29acc1513b09",
        test_ordinal: 1,
        label: "Duplication stays sane",
        test_command: "",
        attempt: 1,
        rendered_command: "analysis:jscpd",
        fixture_values: "{}",
        expected_answer: null,
        state: "resolved",
        outcome: "pass",
        actual:
          '{"clones":8,"duplicated_lines":72,"duplicated_pct":0.73,"tool":"jscpd","total_lines":9860}',
        expected: null,
        exit_code: 0,
        duration_ms: 2070,
        dispatched_at: "2026-08-22T20:59:32.933541Z",
        deadline_at: "2026-08-22T20:59:32.933541Z",
        resolved_at: "2026-08-22T20:59:32.933541Z",
        point_delta: 0,
        updated_at: "2026-08-22T20:59:32.933541Z",
        result: {
          status: "passed",
          expected: null,
          actual:
            '{"clones":8,"duplicated_lines":72,"duplicated_pct":0.73,"tool":"jscpd","total_lines":9860}',
        },
      },
      {
        id: "72ac0777-9d8b-4660-b3aa-b18ae4c05229",
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        task_title: "Track crypto beside the cash",
        task_ordinal: 1,
        adapted_test_id: "38d19499-39cf-455e-96be-4c59ba5e27d0",
        test_ordinal: 2000,
        label: "registered: correctness",
        description:
          "Verify whether the required crypto implementation and completion flag exist in the committed snapshot.",
        test_command:
          "# PROBE registered by judge 'correctness': Verify whether the required crypto implementation and completion flag exist in the committed snapshot.\nprintf 'flag='; test -f .ololo/money-tracker-crypto-done.md && echo yes || echo no; printf 'crypto_refs='; grep -RilE 'BTC|USDC|holding|unrealised|realised|crypto' src tests .ololo 2>/dev/null | tr '\\n' ' '; echo",
        attempt: 1,
        rendered_command:
          "# PROBE registered by judge 'correctness': Verify whether the required crypto implementation and completion flag exist in the committed snapshot.\nprintf 'flag='; test -f .ololo/money-tracker-crypto-done.md && echo yes || echo no; printf 'crypto_refs='; grep -RilE 'BTC|USDC|holding|unrealised|realised|crypto' src tests .ololo 2>/dev/null | tr '\\n' ' '; echo",
        fixture_values: "{}",
        expected_answer: "exit_code === 0 && result.includes('flag=no')",
        state: "resolved",
        outcome: "pass",
        actual:
          "flag=no\ncrypto_refs=src/domain/book.ts src/receipts/files.ts tests/accounts.store.test.ts tests/book.test.ts .ololo/tmp/smoke.log .ololo/artifacts/9a2cf163-fee5-45cb-8075-c639c34700ce/desktop.png",
        expected: "exit_code === 0 && result.includes('flag=no')",
        exit_code: 0,
        duration_ms: 12,
        dispatched_at: "2026-08-22T20:59:40.714322Z",
        deadline_at: "2026-08-22T20:59:40.714322Z",
        resolved_at: "2026-08-22T20:59:40.714322Z",
        point_delta: 0,
        updated_at: "2026-08-22T20:59:40.714322Z",
        result: {
          status: "passed",
          expected: "exit_code === 0 && result.includes('flag=no')",
          actual:
            "flag=no\ncrypto_refs=src/domain/book.ts src/receipts/files.ts tests/accounts.store.test.ts tests/book.test.ts .ololo/tmp/smoke.log .ololo/artifacts/9a2cf163-fee5-45cb-8075-c639c34700ce/desktop.png",
        },
      },
    ],
  }),
);

const judgeResultsByTask = new Map(
  Object.entries({
    "19d84f07-fb4f-407c-998f-b5880078895f": [
      {
        task_id: "19d84f07-fb4f-407c-998f-b5880078895f",
        judge_slug: "correctness",
        judge_name: "Correctness",
        rating: 10.0,
        feedback:
          "The carry-forward is complete and coherent. AGENTS.md still declares the required `stack:`, `run:`, and `test:` commands (AGENTS.md:1-4), and the committed smoke log records 147 passing tests with zero failures (.ololo/tmp/smoke.log). The participant\u2019s completion notes are accurate: the earlier ledger, receipts, and currencies functionality is present and documented in the three `.ololo/*-done.md` files, with matching implementations across `src/domain`, `src/storage`, `src/receipts`, `src/http`, and `src/web`. The required currency behaviors are explicitly covered by passing tests: the March book balance, exact USD retention/reporting, and missing-rate refusal appear in the smoke log and are implemented by `totals`/`convert` (`src/domain/book.ts:77-119`, `src/domain/rates.ts:174-224`). No crypto or unrelated new feature was added, so this setup rung preserves the prior product without regression.",
        point_delta: 10,
        created_at: "2026-08-22T20:18:43.968244Z",
        duration_ms: 11554,
      },
    ],
    "c5c47937-ce66-4c19-b2eb-030da0257b8d": [
      {
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        judge_slug: "creativity",
        judge_name: "Creativity",
        rating: 10.0,
        feedback:
          "The existing product demonstrates careful usability and honest currency handling, but the visible submission does not show an extra crypto-specific capability beyond the requested brief; notably, the required crypto completion file is also absent from the committed files.",
        point_delta: 10,
        created_at: "2026-08-22T20:59:32.960298Z",
        duration_ms: 16868,
      },
      {
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        judge_slug: "test-quality",
        judge_name: "Test Quality",
        rating: 4.0,
        feedback:
          "Add dedicated tests for the crypto scenarios, asserting precise amounts, valuation on different dates, partial sales, and handling of missing price data.",
        point_delta: 4,
        created_at: "2026-08-22T20:59:32.970313Z",
        duration_ms: 2405,
      },
      {
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        judge_slug: "code-quality",
        judge_name: "Code Quality",
        rating: 21.0,
        feedback:
          "The implementation shows strong naming discipline and modular design, with clear handling of rates and monetary amounts. It remains maintainable, though a few large files could be split further for even easier onboarding.",
        point_delta: 21,
        created_at: "2026-08-22T20:59:32.971881Z",
        duration_ms: 3327,
      },
      {
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        judge_slug: "ux-review",
        judge_name: "UX Review",
        rating: 18.0,
        feedback:
          "The existing ledger, accounts, rates, and receipt surfaces look clear and responsive, but visual review found no crypto holdings surface. More importantly, .ololo/money-tracker-crypto-done.md is missing, so the task is not complete.",
        point_delta: 18,
        created_at: "2026-08-22T20:59:32.975768Z",
        duration_ms: 17980,
      },
      {
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        judge_slug: "data",
        judge_name: "Data",
        rating: 22.0,
        feedback:
          "The repository keeps price data in a single database table and uses a dedicated Rates module for all conversions, meeting most data\u2011handling criteria. Minor points are deducted for occasional direct rate access that mixes data logic with higher\u2011level code, preventing a perfect separation score.",
        point_delta: 22,
        created_at: "2026-08-22T20:59:32.976582Z",
        duration_ms: 3731,
      },
      {
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        judge_slug: "correctness",
        judge_name: "Correctness",
        rating: 4.0,
        feedback:
          "The earlier money-tracker functionality is present and polished, but this task\u2019s required crypto product was not implemented: no completion flag, holdings, pinned arithmetic, selling, or fallback crypto pricing.",
        point_delta: 4,
        created_at: "2026-08-22T20:59:32.978239Z",
        duration_ms: 13719,
      },
      {
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        judge_slug: "agentic",
        judge_name: "Agentic",
        rating: 6.0,
        feedback:
          "The project includes a solid AGENTS.md that tells an agent what to do and how to verify, and npm scripts give basic automation. However, there are no guardrails like pre\u2011commit hooks, and the overall repository is larger than needed for the specific crypto scenario, limiting the agentic effectiveness.",
        point_delta: 6,
        created_at: "2026-08-22T20:59:32.991077Z",
        duration_ms: 2416,
      },
      {
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        judge_slug: "architecture",
        judge_name: "Architecture",
        rating: 23.0,
        feedback:
          "The codebase is well\u2011architected with clean separation between domain logic, storage, API, and UI layers, making it easy to test and extend.",
        point_delta: 23,
        created_at: "2026-08-22T20:59:32.991526Z",
        duration_ms: 3674,
      },
    ],
  }),
);
const judgeStatusesByTask = new Map(
  Object.entries({
    "19d84f07-fb4f-407c-998f-b5880078895f": [
      {
        task_id: "19d84f07-fb4f-407c-998f-b5880078895f",
        judge_slug: "correctness",
        judge_name: "Correctness",
        status: "scored",
        error: null,
        updated_at: "2026-08-22T20:18:55.578350Z",
        judge_result_id: "b3170050-b56b-410e-9af5-b4c230ea6aa3",
      },
      {
        task_id: "19d84f07-fb4f-407c-998f-b5880078895f",
        judge_slug: "general",
        judge_name: "The Debrief",
        status: "scored",
        error: null,
        updated_at: "2026-08-22T20:59:52.330161Z",
        judge_result_id: "5d7e2265-1eb1-48f5-aa48-c86846080d46",
      },
    ],
    "c5c47937-ce66-4c19-b2eb-030da0257b8d": [
      {
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        judge_slug: "agentic",
        judge_name: "Agentic",
        status: "scored",
        error: null,
        updated_at: "2026-08-22T20:59:35.470597Z",
        judge_result_id: "d47bfec0-b60b-4601-8be2-41e8f30594d1",
      },
      {
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        judge_slug: "architecture",
        judge_name: "Architecture",
        status: "scored",
        error: null,
        updated_at: "2026-08-22T20:59:36.725606Z",
        judge_result_id: "7232502a-479e-4ca5-9b32-06ca2cbdc30f",
      },
      {
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        judge_slug: "code-quality",
        judge_name: "Code Quality",
        status: "scored",
        error: null,
        updated_at: "2026-08-22T20:59:36.360878Z",
        judge_result_id: "f9353e7e-b1bd-4bbc-92a0-262a0400b424",
      },
      {
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        judge_slug: "correctness",
        judge_name: "Correctness",
        status: "scored",
        error: null,
        updated_at: "2026-08-22T20:59:46.781672Z",
        judge_result_id: "a7fa42ff-42cc-44ed-bce6-c870c4bacda2",
      },
      {
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        judge_slug: "creativity",
        judge_name: "Creativity",
        status: "scored",
        error: null,
        updated_at: "2026-08-22T20:59:49.931122Z",
        judge_result_id: "74b3759d-e74c-470f-a2de-f63a5cf4eb58",
      },
      {
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        judge_slug: "data",
        judge_name: "Data",
        status: "scored",
        error: null,
        updated_at: "2026-08-22T20:59:36.768018Z",
        judge_result_id: "b5ef7eae-6319-4f2e-96b5-b3c268d6f935",
      },
      {
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        judge_slug: "test-quality",
        judge_name: "Test Quality",
        status: "scored",
        error: null,
        updated_at: "2026-08-22T20:59:35.432663Z",
        judge_result_id: "951d04b3-9c3c-4569-83e0-6299a4103f91",
      },
      {
        task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        judge_slug: "ux-review",
        judge_name: "UX Review",
        status: "scored",
        error: null,
        updated_at: "2026-08-22T20:59:51.041564Z",
        judge_result_id: "a7b4deb6-5542-4ef9-9c3a-93be0b402341",
      },
    ],
  }),
);
const evaluationsByTask = new Map(
  Object.entries({
    "c5c47937-ce66-4c19-b2eb-030da0257b8d": {
      task_id: "c5c47937-ce66-4c19-b2eb-030da0257b8d",
      criteria: [
        {
          key: "product",
          title: "Matches the brief",
          weight: 0.25,
          scores: [
            {
              judge_slug: "correctness",
              score: 1.0,
              rationale:
                "The submitted product is the earlier cash, currency, account, and receipt tracker, but the required crypto feature is absent. The completion flag named by the task is missing, and the repository contains no crypto holding, buy/sell, valuation, realised/unrealised result, or eight-decimal holding implementation. The committed smoke log reports 147 tests, but none cover the crypto scenarios. The visible screenshots likewise show only Accounts, Rates, Receipts, and cash ledger views, with no wallet or crypto UI.",
            },
          ],
        },
        {
          key: "architecture",
          title: "Architecture",
          weight: 0.1,
          scores: [
            {
              judge_slug: "architecture",
              score: 9.0,
              rationale:
                "The repository follows a clear layered architecture. Domain concerns (e.g. src/domain/*.ts) are isolated from persistence (src/storage/*.ts), HTTP handling (src/http/*.ts), and presentation (src/web/*.ts). Each file has a focused responsibility \u2013 e.g., money.ts defines currency handling, rates.ts handles conversion logic, account.ts deals with account validation. Dependencies flow inward: higher\u2011level layers import from lower\u2011level domain modules, but domain modules do not depend on storage or UI, avoiding circular coupling. The structure is proportional to the problem size: the project is a full\u2011featured money\u2011tracking product, so the multiple layers and separate modules are justified, and no unnecessary indirection is present. This reflects good separation of concerns, clear component boundaries, proper dependency direction, and appropriate proportionality.",
            },
          ],
        },
        {
          key: "data",
          title: "Data layer",
          weight: 0.1,
          scores: [
            {
              judge_slug: "data",
              score: 8.5,
              rationale:
                "All price information is stored in the `rates` SQLite table (see `src/storage/migrations/004_currencies.sql`) and accessed through the `Rates` class (`src/domain/rates.ts`) and the `rateStore` wrapper (`src/storage/rates.ts`). This provides a single source of truth; there are no hard\u2011coded price literals elsewhere in the code or UI, satisfying the \u2018source of truth\u2019 and \u2018no shadow data\u2019 requirements. Rate parsing, conversion, and missing\u2011rate handling are encapsulated in the domain layer, keeping data handling separate from presentation logic, though a few callers invoke `Rates` directly which slightly lowers the separation score. The implementation uses the supplied price table for calculations (e.g., conversion in `convert` and `rateBetween`), respecting the pinned arithmetic, so the data is honestly sourced. No duplicated copies of the price data are found, and missing rates are reported rather than guessed, matching the task constraints.",
            },
          ],
        },
        {
          key: "ux",
          title: "UI/UX",
          weight: 0.1,
          scores: [
            {
              judge_slug: "ux-review",
              score: 6.0,
              rationale:
                "The delivered screenshots show a coherent, polished money-tracker interface with strong balance hierarchy, clear cards, readable forms, and consistent spacing. However, no crypto holdings surface is visible, so the requested crypto workflow and its key values cannot be visually credited.",
            },
          ],
        },
        {
          key: "accessibility",
          title: "Accessibility",
          weight: 0.05,
          scores: [
            {
              judge_slug: "ux-review",
              score: 7.0,
              rationale:
                "Screenshots show high-contrast dark text, distinct labels, and clear positive/negative states. Supporting markup uses semantic headings, labels, forms, tables, aria labels, and status/alert roles. The committed files nevertheless do not show a crypto-specific surface, and the required completion flag is missing.",
            },
          ],
        },
        {
          key: "mobile",
          title: "Mobile readiness",
          weight: 0.05,
          scores: [
            {
              judge_slug: "ux-review",
              score: 8.0,
              rationale:
                "The attached narrow screenshots show responsive single-column layouts without visible horizontal overflow, with controls and cards reflowing appropriately. The crypto-specific mobile experience cannot be assessed because no crypto screenshot or corresponding surface is present.",
            },
          ],
        },
        {
          key: "cleanliness",
          title: "Code cleanliness",
          weight: 0.05,
          scores: [
            {
              judge_slug: "code-quality",
              score: 8.2,
              rationale:
                "The codebase uses clear, self\u2011describing names (e.g. `parseRate`, `formatRate`, `convertEntries`, `currencyParts`). Constants such as `RATE_SCALE`, `ONE_RATE`, and `DEFAULT_CURRENCY` centralise magic values. There is no dead code or evident copy\u2011paste of large blocks; each module has a focused responsibility. Small helper functions are defined once (e.g. `formatAmount`, `sumMoney`) and reused, avoiding duplication.",
            },
          ],
        },
        {
          key: "maintainability",
          title: "Maintainability",
          weight: 0.05,
          scores: [
            {
              judge_slug: "code-quality",
              score: 7.6,
              rationale:
                "The architecture is modular with low nesting depth and short functions, making it easy for a newcomer to understand. Error handling is explicit (e.g. `AmountError`, `ValidationError`). Magic numbers are named constants, and types are well\u2011defined (`Money`, `Minor`, `Conversion`). Some files are relatively long (e.g. `book.ts`), but they remain logically grouped and documented, supporting safe changes. No hidden side\u2011effects or undocumented behavior appear.",
            },
          ],
        },
        {
          key: "tests",
          title: "Tests",
          weight: 0.1,
          scores: [
            {
              judge_slug: "test-quality",
              score: 1.5,
              rationale:
                "The repository contains a rich suite of tests for entries, accounts, rates, receipts, and overall ledger scenarios, all with concrete assertions. However, none of the test files address the crypto-specific scenarios described in the task (buying fractions, eight\u2011decimal precision, valuation on a later date, partial sale, aggregation with cash, handling missing price). A search of the test directory shows no mentions of BTC, ETH, USDC, or related holdings, indicating that the critical crypto functionality is untested. Consequently, regressions in the newly added crypto features would not be caught by the existing test suite.",
            },
          ],
        },
        {
          key: "agentic",
          title: "Agentic workflow",
          weight: 0.05,
          scores: [
            {
              judge_slug: "agentic",
              score: 4.9,
              rationale:
                "AGENTS.md provides clear instructions for an agent: the stack, how to run (`npm start`) and verify (`npm test`), and constraints (integer amounts, no rounding, rates handling) that match the task requirements. This gives a strong instruction component. Reusable automation exists via npm scripts (`start`, `test`, `samples`), which the agent can invoke, but there are no dedicated skill definitions or higher\u2011level command wrappers. No pre\u2011commit or other automatic guardrails are present, so error\u2011catching is left to manual testing. The repository is sizable for a simple crypto\u2011tracking task, adding unnecessary boilerplate, which reduces proportionality. Overall, the instruction quality is high, automation moderate, hooks absent, and size somewhat disproportionate, leading to a balanced agentic score.",
            },
          ],
        },
        {
          key: "creativity",
          title: "Creativity",
          weight: 0.1,
          scores: [
            {
              judge_slug: "creativity",
              score: 5.0,
              rationale:
                "The delivered artifacts and repository show a strong, thoughtful multi-currency money tracker with accounts, transfers, receipt ingestion, exact money handling, rate-gap messaging, filters, and responsive layouts. However, these are the core product capabilities described by the brief and its earlier parts rather than a clearly additional, unrequested feature for crypto. I found no committed crypto holdings, valuation, sale, or crypto-specific UX implementation to reward beyond the brief.",
            },
          ],
        },
      ],
      deadline_at: "2026-08-22T20:58:17.886444Z",
      measurements: [
        {
          test_ordinal: 1,
          label: "Duplication stays sane",
          at: "2026-08-22T20:59:32.933541Z",
          outcome: "pass",
          result_json: {
            analysis: {
              clones: 8,
              duplicated_lines: 72,
              duplicated_pct: 0.73,
              tool: "jscpd",
              total_lines: 9860,
            },
            snapshot_age_secs: 2473,
            snapshot_commit: "01d03739aa1ca9476aec8092a6ae2b0b976a039d",
            summary: "jscpd: 0.73% duplicated (8 clones)",
          },
        },
        {
          test_ordinal: 2000,
          label: "registered: correctness",
          at: "2026-08-22T20:59:40.714322Z",
          outcome: "pass",
          result_json: {
            snapshot_age_secs: 2483,
            snapshot_commit: "01d03739aa1ca9476aec8092a6ae2b0b976a039d",
            stderr: "",
            timed_out: false,
          },
        },
      ],
      repo_images: [
        "samples/receipts/bistro-2026-03-16.png",
        "samples/receipts/konzum-2026-03-14.png",
      ],
    },
  }),
);
const judgeAvatars = {
  agentic: "https://ik.imagekit.io/tdf7wfnyrgb/judges/judge-avatar.png",
  architecture: "https://ik.imagekit.io/tdf7wfnyrgb/judges/judge-avatar.png",
  "code-quality": "https://ik.imagekit.io/tdf7wfnyrgb/judges/judge-avatar.png",
  correctness: "https://ik.imagekit.io/tdf7wfnyrgb/judges/judge-avatar.png",
  creativity: "https://ik.imagekit.io/tdf7wfnyrgb/judges/judge-avatar.png",
  data: "https://ik.imagekit.io/tdf7wfnyrgb/judges/judge-avatar.png",
  "test-quality": "https://ik.imagekit.io/tdf7wfnyrgb/judges/judge-avatar.png",
  "ux-review": "https://ik.imagekit.io/tdf7wfnyrgb/judges/judge-avatar.png",
};
const report = {
  judge_name: "The Debrief",
  judge_slug: "general",
  markdown:
    '{"built":{"brief":"You completed the base money\u2011tracker product, preserving the ledger, receipt, and currency features from earlier rungs. The repository includes the domain, storage, API, and UI layers, and a smoke log shows all 147 tests passing. No new crypto functionality was added.","tasks":[{"ordinal":0,"note":"carried forward the earlier ledger, receipt and currency implementations and added a smoke log confirming all checks pass"}]},"friction":[{"ordinal":1,"what_happened":"you reached task two but did not submit any code or agent activity","why":"commit_sha is null, agent data is missing, and the task is flagged with no_task_commit and no_agent_stats"}],"judges":[{"judge":"Correctness","good":"the carry\u2011forward is complete and coherent, with a passing smoke log","improve":null},{"judge":"Agentic","good":"solid AGENTS.md and npm scripts give basic automation","improve":"add guardrails such as pre\u2011commit hooks and trim the repository to the crypto scenario"},{"judge":"Architecture","good":"codebase has clean separation between domain, storage, API, and UI layers","improve":null},{"judge":"Code Quality","good":"strong naming discipline and modular design across the codebase","improve":"split a few large files to make onboarding easier"},{"judge":"Correctness","good":"the earlier money\u2011tracker functionality is present and polished","improve":"implement the required crypto holdings, arithmetic, and pricing logic"},{"judge":"Creativity","good":"the product demonstrates careful usability and honest currency handling","improve":"add a crypto\u2011specific capability beyond the existing features"},{"judge":"Data","good":"price data lives in a single table and is accessed via the Rates module","improve":"avoid direct rate access that mixes data logic with higher\u2011level code"},{"judge":"Test Quality","good":"the existing test suite covers many core scenarios","improve":"add dedicated tests for crypto scenarios, including valuation, partial sales, and missing price handling"},{"judge":"UX Review","good":"ledger, accounts, rates, and receipt surfaces are clear and responsive","improve":"provide a UI for crypto holdings and include the missing .ololo/money-tracker-crypto-done.md documentation"}],"improve":["implement the crypto holdings feature with domain, storage, API, and UI components, because the task has no commit and judges note the required crypto product is missing","add a .ololo/money-tracker-crypto-done.md file documenting the crypto implementation, as UX Review and Correctness judges point out its absence","write dedicated crypto tests asserting amounts, valuations on different dates, partial sales, and missing price handling, as recommended by the Test Quality judge"]}',
  document: {
    built: {
      brief:
        "You completed the base money\u2011tracker product, preserving the ledger, receipt, and currency features from earlier rungs. The repository includes the domain, storage, API, and UI layers, and a smoke log shows all 147 tests passing. No new crypto functionality was added.",
      tasks: [
        {
          ordinal: 0,
          note: "carried forward the earlier ledger, receipt and currency implementations and added a smoke log confirming all checks pass",
        },
      ],
    },
    friction: [
      {
        ordinal: 1,
        what_happened: "you reached task two but did not submit any code or agent activity",
        why: "commit_sha is null, agent data is missing, and the task is flagged with no_task_commit and no_agent_stats",
      },
    ],
    judges: [
      {
        judge: "Correctness",
        good: "the carry\u2011forward is complete and coherent, with a passing smoke log",
        improve: null,
      },
      {
        judge: "Agentic",
        good: "solid AGENTS.md and npm scripts give basic automation",
        improve:
          "add guardrails such as pre\u2011commit hooks and trim the repository to the crypto scenario",
      },
      {
        judge: "Architecture",
        good: "codebase has clean separation between domain, storage, API, and UI layers",
        improve: null,
      },
      {
        judge: "Code Quality",
        good: "strong naming discipline and modular design across the codebase",
        improve: "split a few large files to make onboarding easier",
      },
      {
        judge: "Correctness",
        good: "the earlier money\u2011tracker functionality is present and polished",
        improve: "implement the required crypto holdings, arithmetic, and pricing logic",
      },
      {
        judge: "Creativity",
        good: "the product demonstrates careful usability and honest currency handling",
        improve: "add a crypto\u2011specific capability beyond the existing features",
      },
      {
        judge: "Data",
        good: "price data lives in a single table and is accessed via the Rates module",
        improve: "avoid direct rate access that mixes data logic with higher\u2011level code",
      },
      {
        judge: "Test Quality",
        good: "the existing test suite covers many core scenarios",
        improve:
          "add dedicated tests for crypto scenarios, including valuation, partial sales, and missing price handling",
      },
      {
        judge: "UX Review",
        good: "ledger, accounts, rates, and receipt surfaces are clear and responsive",
        improve:
          "provide a UI for crypto holdings and include the missing .ololo/money-tracker-crypto-done.md documentation",
      },
    ],
    improve: [
      "implement the crypto holdings feature with domain, storage, API, and UI components, because the task has no commit and judges note the required crypto product is missing",
      "add a .ololo/money-tracker-crypto-done.md file documenting the crypto implementation, as UX Review and Correctness judges point out its absence",
      "write dedicated crypto tests asserting amounts, valuations on different dates, partial sales, and missing price handling, as recommended by the Test Quality judge",
    ],
  },
  created_at: "2026-08-22T20:59:52.330161Z",
};

const meta = {
  title: "Sessions/SessionReportPanel",
  component: SessionReportPanel,
  parameters: { layout: "padded" },
} satisfies Meta<SessionReportPanel>;

export default meta;
type Story = StoryObj<typeof meta>;

export const FinishedSession: Story = {
  args: {
    report,
    sessionFinished: true,
    tasks,
    probesByTask,
    judgeResultsByTask,
    judgeStatusesByTask,
    evaluationsByTask,
    judgeAvatars,
    sessionCode: "3WTCA2",
    playerId: "3067b6a2-a792-4fe4-88d5-39c030348712",
    score: 128,
    rank: 1,
    totalTasks: 2,
  },
};

/** No verdicts reached the page — the reporter's summary stands alone. */
export const ReportOnly: Story = {
  args: {
    ...FinishedSession.args,
    judgeResultsByTask: new Map(),
    judgeStatusesByTask: new Map(),
    evaluationsByTask: new Map(),
  },
};

/** The captures a session leaves behind: a gallery that opens fullscreen,
 *  with ‹ › and the arrow keys walking it. The thumbnails themselves 404 in
 *  the workbench — the panel addresses them through the session's artifacts
 *  endpoint — so this story is for the strip's layout and the overlay's
 *  chrome, not the pictures. */
const CAPTURES = [
  {
    probe_id: "shot-receipt",
    content_type: "image/png",
    label: ".ololo/artifacts/8ddccd0e/receipt-desktop.png",
    file_count: 2,
    files: [
      ".ololo/artifacts/8ddccd0e/receipt-desktop.png",
      ".ololo/artifacts/8ddccd0e/receipt-mobile.png",
    ],
  },
  {
    probe_id: "shot-book",
    content_type: "image/png",
    label: ".ololo/artifacts/02fa25e2/book-desktop.png",
    file_count: 2,
    files: [
      ".ololo/artifacts/02fa25e2/book-desktop.png",
      ".ololo/artifacts/02fa25e2/book-mobile.png",
    ],
  },
];

export const WithCaptures: Story = {
  args: {
    ...FinishedSession.args,
    evaluationsByTask: new Map([
      [
        "c5c47937-ce66-4c19-b2eb-030da0257b8d",
        {
          ...evaluationsByTask.get("c5c47937-ce66-4c19-b2eb-030da0257b8d"),
          artifacts: CAPTURES,
        },
      ],
    ]),
  },
};
