<script lang="ts">
  import { onMount } from 'svelte'

  // Animated walkthrough of one ololo session, in five looping phases.
  // Each phase highlights one beat of the game: start/join, agent builds,
  // live checks, judge verdicts, leaderboard. Phases auto-advance; the
  // step cards double as manual controls for presentations.
  //
  // `arena` marks an edition with the global Arena ladder: phase 5 then
  // talks about the rating that outlives the session. Without it (the open
  // platform has no Arena) the finale stays inside the session — top the
  // board, win the round.
  let { arena = false }: { arena?: boolean } = $props()

  const PHASE_MS = 4600
  const TITLES = $derived<Record<number, string>>({
    1: 'Start a live session',
    2: 'ololo sends the task — your agent builds',
    3: 'Get checked as you go',
    4: 'Hear from the judges',
    5: arena ? 'Watch your rating grow' : 'Top the leaderboard',
  })

  let phase = $state(1)
  let playing = $state(true)
  let reduced = $state(false)
  let timer: ReturnType<typeof setInterval> | null = null

  const seen = $derived([1, 2, 3, 4, 5].map((n) => phase >= n))

  function start() {
    if (timer) clearInterval(timer)
    timer = setInterval(() => {
      phase = phase >= 5 ? 1 : phase + 1
    }, PHASE_MS)
    playing = true
  }

  function stop() {
    if (timer) clearInterval(timer)
    timer = null
    playing = false
  }

  function jump(n: number) {
    phase = n
    if (playing) start()
  }

  onMount(() => {
    reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches
    if (reduced) {
      // Everything visible, nothing moving — the CSS media query freezes
      // the loops; here we just land on the final, complete scene.
      phase = 5
      playing = false
    } else {
      start()
    }
    return () => {
      if (timer) clearInterval(timer)
    }
  })

  const steps = $derived([
    {
      n: 1,
      title: TITLES[1],
      html: true,
      text: '',
    },
    {
      n: 2,
      title: TITLES[2],
      html: false,
      text: 'ololo deals the same task to every player. Your AI agent builds it right on your machine.',
    },
    {
      n: 3,
      title: TITLES[3],
      html: false,
      text: 'ololo runs quick live checks on every task and scores you in real time.',
    },
    {
      n: 4,
      title: TITLES[4],
      html: false,
      text: 'AI judges review your work — correctness, quality, creativity — and tell you why.',
    },
    {
      n: 5,
      title: TITLES[5],
      html: false,
      text: arena
        ? 'Points land on the live leaderboard. Every session levels up your skills.'
        : 'Points land on the live leaderboard as checks pass and judges score — winner takes the round.',
    },
  ])
</script>

<section aria-label="Animated walkthrough of one ololo session">
  <div class="flex items-center justify-between gap-3 px-1 pb-3">
    <p class="font-heading text-[15px] font-bold text-[#363636]">
      <span
        class="mr-2 inline-flex h-6 w-6 items-center justify-center rounded-full bg-[#0269fb] align-[-5px] text-[13px] font-bold text-white"
      >
        {phase}
      </span>
      {TITLES[phase]}
    </p>
    <button
      type="button"
      class="rounded-btn border border-[#0269fb] bg-white px-5 py-1.5 text-[13px] font-semibold text-[#0269fb] transition-colors hover:bg-blue-50"
      onclick={() => (playing ? stop() : start())}
    >
      {playing ? 'Pause' : 'Play'}
    </button>
  </div>

  <figure class="m-0">
    <svg
      viewBox="0 0 960 430"
      role="img"
      aria-label="One ololo session: a player starts it with 'ololo start weather-widget', rivals join with 'ololo join QW3RT', agents build, ololo runs live checks, AI judges review the work, and points land on a live leaderboard."
      class="scene block h-auto w-full"
      class:phase-1={phase === 1}
      class:phase-2={phase === 2}
      class:phase-3={phase === 3}
      class:phase-4={phase === 4}
      class:phase-5={phase === 5}
      class:seen-1={seen[0]}
      class:seen-2={seen[1]}
      class:seen-3={seen[2]}
      class:seen-4={seen[3]}
      class:seen-5={seen[4]}
    >
      <!-- ===== wires ===== -->
      <path class="wire-path wire-probe" d="M420,178 C340,158 316,132 246,120" />
      <path class="wire-path wire-probe" d="M420,205 C348,218 322,248 246,258" />
      <path class="wire-path wire-probe" d="M420,220 C352,242 326,320 246,338" />
      <path class="wire-path wire-judge" d="M540,178 C620,150 636,124 700,112" />
      <path class="wire-path wire-board" d="M540,222 C620,252 636,308 700,320" />

      <!-- ===== you (left top) ===== -->
      <g class="g-you">
        <rect class="card-shape" x="42" y="52" width="204" height="136" rx="8" />
        <rect x="42" y="52" width="204" height="26" rx="8" fill="#eef4fd" />
        <rect x="42" y="66" width="204" height="12" fill="#eef4fd" />
        <circle cx="58" cy="65" r="3.5" fill="#fb341c" opacity="0.85" />
        <circle cx="70" cy="65" r="3.5" fill="#f6b73c" opacity="0.85" />
        <circle cx="82" cy="65" r="3.5" fill="#14934a" opacity="0.85" />
        <text class="tiny-mono" x="144" y="69" text-anchor="middle">you × your agent</text>
        <text class="cmd cmd-you cmd-type" x="56" y="100">
          <tspan class="prompt-ch">$</tspan> ololo start weather-widget</text
        >
        <g class="g-task">
          <rect x="56" y="110" width="150" height="16" rx="4" fill="#eef4fd" stroke="#dce9fc" />
          <rect x="56" y="110" width="3" height="16" fill="#0269fb" />
          <text x="65" y="121" class="task-line">Task 1 · current weather card</text>
        </g>
        <g class="g-code">
          <rect class="code-line cl1" x="56" y="136" width="120" height="6" rx="3" fill="#dce9fc" />
          <rect class="code-line cl2" x="56" y="148" width="152" height="6" rx="3" fill="#dce9fc" />
          <rect class="code-line cl3" x="56" y="160" width="96" height="6" rx="3" fill="#dce9fc" />
          <rect class="code-line cl4" x="56" y="172" width="134" height="6" rx="3" fill="#dce9fc" />
          <rect class="cursor" x="196" y="168" width="7" height="11" fill="#0269fb" />
        </g>
        <g class="flash f1">
          <rect x="196" y="163" width="40" height="18" rx="9" fill="#dcf5e7" />
          <text x="216" y="176" text-anchor="middle" class="chip-ok">✓ +3</text>
        </g>
      </g>

      <!-- ===== rivals (left bottom) ===== -->
      <g class="g-rivals">
        <rect class="card-shape" x="42" y="226" width="204" height="62" rx="8" />
        <circle cx="70" cy="257" r="14" fill="#dce9fc" />
        <text x="70" y="262" text-anchor="middle" class="avatar-ch">K</text>
        <text class="label" x="94" y="253">kira_dev</text>
        <text class="cmd cmd-r1 cmd-type small-cmd" x="94" y="270">
          <tspan class="prompt-ch">$</tspan> ololo join QW3RT</text
        >
        <g class="flash f2">
          <rect x="192" y="234" width="40" height="18" rx="9" fill="#dcf5e7" />
          <text x="212" y="247" text-anchor="middle" class="chip-ok">✓ +3</text>
        </g>

        <rect class="card-shape" x="42" y="306" width="204" height="62" rx="8" />
        <circle cx="70" cy="337" r="14" fill="#dce9fc" />
        <text x="70" y="342" text-anchor="middle" class="avatar-ch">M</text>
        <text class="label" x="94" y="333">max_and_bot</text>
        <text class="cmd cmd-r2 cmd-type small-cmd" x="94" y="350">
          <tspan class="prompt-ch">$</tspan> ololo join QW3RT</text
        >
        <g class="flash f3">
          <rect x="192" y="314" width="40" height="18" rx="9" fill="#dcf5e7" />
          <text x="212" y="327" text-anchor="middle" class="chip-ok">✓ +2</text>
        </g>
      </g>

      <!-- ===== hub (center): the ololo logo runs the session,
                 the mascot hosts it from below ===== -->
      <g class="g-hub">
        <circle class="pulse p1" cx="480" cy="200" r="58" />
        <circle class="pulse p2" cx="480" cy="200" r="58" />
        <circle cx="480" cy="200" r="56" fill="#ffffff" stroke="#0269fb" stroke-width="2" />
        <image href="/logo.svg" x="458" y="160" width="44" height="44" aria-hidden="true" />
        <text class="small" x="480" y="228" text-anchor="middle">weather-widget</text>
        <g>
          <rect x="440" y="122" width="80" height="19" rx="9.5" fill="#ffe8e5" />
          <circle cx="452" cy="131.5" r="3" fill="#fb341c" />
          <text x="484" y="135" text-anchor="middle" class="pill-running">RUNNING</text>
        </g>
        <g>
          <rect x="378" y="268" width="102" height="20" rx="4" fill="#f2f5fa" />
          <text x="429" y="282" text-anchor="middle">
            <tspan class="chip-label">join code</tspan>
            <tspan class="chip-code">QW3RT</tspan>
          </text>
          <rect x="488" y="268" width="94" height="20" rx="4" fill="#f2f5fa" />
          <text x="535" y="282" text-anchor="middle">
            <tspan class="chip-label">⏱</tspan>
            <tspan class="chip-code">42:17</tspan>
            <tspan class="chip-label">left</tspan>
          </text>
        </g>
        <image href="/maskot.png" x="424" y="300" width="112" height="112" aria-hidden="true" />
      </g>

      <!-- ===== judges (right top) ===== -->
      <g class="g-judges">
        <rect class="card-shape" x="700" y="46" width="222" height="146" rx="8" />
        <text class="label" x="716" y="72">AI judges</text>
        <text class="small" x="716" y="88">they review — and explain why</text>
        <g class="judge-row jr1">
          <rect x="716" y="98" width="190" height="24" rx="12" fill="#dce9fc" />
          <text x="730" y="114" class="row-deep">✓ Correctness</text>
          <text x="892" y="114" text-anchor="end" class="row-deep">9/10</text>
        </g>
        <g class="judge-row jr2">
          <rect x="716" y="128" width="190" height="24" rx="12" fill="#dce9fc" />
          <text x="730" y="144" class="row-deep">✓ Code quality</text>
          <text x="892" y="144" text-anchor="end" class="row-deep">8/10</text>
        </g>
        <g class="judge-row jr3">
          <rect x="716" y="158" width="190" height="24" rx="12" fill="#dce9fc" />
          <text x="730" y="174" class="row-deep">★ Creativity</text>
          <text x="892" y="174" text-anchor="end" class="row-deep">+5</text>
        </g>
      </g>

      <!-- ===== leaderboard (right bottom) ===== -->
      <g class="g-board">
        <rect class="card-shape" x="700" y="238" width="222" height="146" rx="8" />
        <text class="label" x="716" y="264">Live leaderboard</text>
        <g class="board-row br1">
          <rect x="710" y="272" width="202" height="28" rx="6" fill="#eef4fd" />
          <text x="720" y="291" class="medal">🥇</text>
          <text x="744" y="291" class="row-you">you</text>
          <text x="902" y="291" text-anchor="end" class="score-you">128 pts</text>
        </g>
        <g class="board-row br2">
          <text x="720" y="319" class="medal">🥈</text>
          <text x="744" y="319" class="row-mut">kira_dev</text>
          <text x="902" y="319" text-anchor="end" class="score-mut">95 pts</text>
        </g>
        <g class="board-row br3">
          <text x="720" y="347" class="medal">🥉</text>
          <text x="744" y="347" class="row-mut">max_and_bot</text>
          <text x="902" y="347" text-anchor="end" class="score-mut">67 pts</text>
        </g>
        <g class="popchip">
          <line x1="716" y1="357" x2="906" y2="357" stroke="#e7eaee" />
          {#if arena}
            <text x="716" y="374" class="foot-label">Your Arena rating</text>
            <text x="906" y="374" text-anchor="end" class="foot-delta">+18 ↑</text>
          {:else}
            <text x="716" y="374" class="foot-label">Session result</text>
            <text x="906" y="374" text-anchor="end" class="foot-delta">🏆 you win</text>
          {/if}
        </g>
      </g>

      <!-- ===== task cards flying hub → players (phase 2) ===== -->
      <g class="task-fly">
        <animateMotion dur="2.2s" repeatCount="indefinite" path="M420,178 C340,158 316,132 246,120" />
        <rect x="-23" y="-9" width="46" height="18" rx="9" fill="#0269fb" />
        <text x="0" y="3.5" text-anchor="middle" class="task-fly-label">Task 1</text>
      </g>
      <g class="task-fly">
        <animateMotion
          dur="2.2s"
          begin="0.5s"
          repeatCount="indefinite"
          path="M420,205 C348,218 322,248 246,258"
        />
        <rect x="-23" y="-9" width="46" height="18" rx="9" fill="#0269fb" />
        <text x="0" y="3.5" text-anchor="middle" class="task-fly-label">Task 1</text>
      </g>
      <g class="task-fly">
        <animateMotion
          dur="2.2s"
          begin="1s"
          repeatCount="indefinite"
          path="M420,220 C352,242 326,320 246,338"
        />
        <rect x="-23" y="-9" width="46" height="18" rx="9" fill="#0269fb" />
        <text x="0" y="3.5" text-anchor="middle" class="task-fly-label">Task 1</text>
      </g>

      <!-- ===== moving dots ===== -->
      <circle class="dot dot-blue dot-probe" r="5">
        <animateMotion dur="1.8s" repeatCount="indefinite" path="M420,178 C340,158 316,132 246,120" />
      </circle>
      <circle class="dot dot-blue dot-probe" r="5">
        <animateMotion
          dur="1.8s"
          begin="0.6s"
          repeatCount="indefinite"
          path="M420,205 C348,218 322,248 246,258"
        />
      </circle>
      <circle class="dot dot-blue dot-probe" r="5">
        <animateMotion
          dur="1.8s"
          begin="1.2s"
          repeatCount="indefinite"
          path="M420,220 C352,242 326,320 246,338"
        />
      </circle>
      <circle class="dot dot-ok dot-probe" r="4">
        <animateMotion
          dur="1.8s"
          begin="0.9s"
          repeatCount="indefinite"
          path="M246,120 C316,132 340,158 420,178"
        />
      </circle>
      <circle class="dot dot-ok dot-probe" r="4">
        <animateMotion
          dur="1.8s"
          begin="1.5s"
          repeatCount="indefinite"
          path="M246,258 C322,248 348,218 420,205"
        />
      </circle>
      <circle class="dot dot-blue dot-judge" r="5">
        <animateMotion dur="1.9s" repeatCount="indefinite" path="M540,178 C620,150 636,124 700,112" />
      </circle>
      <circle class="dot dot-ok dot-judge" r="4">
        <animateMotion
          dur="1.9s"
          begin="0.95s"
          repeatCount="indefinite"
          path="M700,112 C636,124 620,150 540,178"
        />
      </circle>
      <circle class="dot dot-blue dot-board" r="5">
        <animateMotion dur="1.6s" repeatCount="indefinite" path="M540,222 C620,252 636,308 700,320" />
      </circle>
    </svg>
    <figcaption class="sr-only">
      One ololo session, animated: a player starts it with "ololo start weather-widget", rivals join
      with "ololo join QW3RT", agents build, ololo runs live checks, AI judges review the work, and
      the leaderboard updates in real time.
    </figcaption>
  </figure>

  <div
    class="mt-4 grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-5"
    role="tablist"
    aria-label="Session walkthrough steps"
  >
    {#each steps as step (step.n)}
      <button
        type="button"
        role="tab"
        aria-selected={phase === step.n}
        onclick={() => jump(step.n)}
        class="group relative flex flex-col overflow-hidden rounded-[8px] bg-white p-[16px] pb-[18px] text-left transition-shadow hover:shadow-[0_6px_32px_0_rgba(19,101,218,0.16)]
          {phase === step.n
          ? 'shadow-[0_6px_32px_0_rgba(19,101,218,0.16)] ring-2 ring-inset ring-[#0269fb]'
          : ''}"
      >
        <span class="mb-[8px] flex items-center gap-[8px]">
          <span
            class="flex h-[24px] w-[24px] shrink-0 items-center justify-center rounded-full text-[12px] font-bold transition-colors
              {phase === step.n ? 'bg-[#0269fb] text-white' : 'bg-[#dce9fc] text-[#3061ac]'}"
          >
            {step.n}
          </span>
          <span
            class="text-[11px] font-bold uppercase tracking-[0.08em]
              {phase === step.n ? 'text-[#0269fb]' : 'text-[#3061ac]'}"
          >
            Step {step.n} <span class="text-[#b8d4f8]">/ 5</span>
          </span>
        </span>
        <span class="block font-heading text-[14.5px] font-bold leading-[1.33] text-[#363636]">
          {step.title}
        </span>
        <span class="mt-[6px] block text-[13px] leading-[1.55] text-[#6b7a90]">
          {#if step.html}
            <code class="cmd-pill">ololo start weather-widget</code>
            opens it — rivals hop in with
            <code class="cmd-pill">ololo join QW3RT</code>.
          {:else}
            {step.text}
          {/if}
        </span>
        <span
          class="mt-auto flex items-center pt-[10px] text-[13px] font-bold leading-none text-[#0269fb] transition-opacity
            {phase === step.n ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'}"
          aria-hidden="true"
        >
          {phase === step.n ? 'Watching' : 'Watch'}
          <svg
            class="ml-1 fill-[#0269fb]"
            xmlns="http://www.w3.org/2000/svg"
            width="9"
            height="10"
            viewBox="0 0 306 306"
          >
            <path d="M94.35 0l-35.7 35.7L175.95 153 58.65 270.3l35.7 35.7 153-153z" />
          </svg>
        </span>
        {#if phase === step.n && playing && !reduced}
          {#key phase}
            <span class="progress" style="animation-duration: {PHASE_MS}ms;"></span>
          {/key}
        {/if}
      </button>
    {/each}
  </div>
</section>

<style>
  .scene {
    font-family: inherit;
  }
  .cmd-pill {
    font-family:
      ui-monospace, 'SF Mono', Menlo, Consolas, monospace;
    font-size: 11px;
    font-weight: 600;
    color: #3061ac;
    background: #dce9fc;
    border-radius: 8px;
    padding: 1px 6px;
    white-space: nowrap;
  }
  .progress {
    position: absolute;
    left: 0;
    bottom: 0;
    height: 3px;
    width: 100%;
    background: #0269fb;
    transform: scaleX(0);
    transform-origin: left;
    animation-name: fill;
    animation-timing-function: linear;
    animation-fill-mode: forwards;
  }
  @keyframes fill {
    to {
      transform: scaleX(1);
    }
  }

  /* ---------- SVG scene ---------- */
  .card-shape {
    fill: #ffffff;
    filter: drop-shadow(0 1px 2px rgba(16, 24, 40, 0.06));
  }
  .label {
    font-size: 13px;
    font-weight: 700;
    fill: #363636;
  }
  .small {
    font-size: 11px;
    font-weight: 600;
    fill: #9ea7b6;
  }
  .cmd {
    font-size: 10.5px;
    font-family: ui-monospace, 'SF Mono', Menlo, Consolas, monospace;
    fill: #363636;
  }
  .cmd.small-cmd {
    font-size: 10px;
  }
  .prompt-ch {
    fill: #0269fb;
  }
  .tiny-mono {
    font-size: 10px;
    fill: #9ea7b6;
    font-family: ui-monospace, Menlo, Consolas, monospace;
  }
  .avatar-ch {
    font-size: 12px;
    font-weight: 700;
    fill: #0269fb;
  }
  .chip-ok {
    font-size: 11px;
    font-weight: 700;
    fill: #14934a;
  }
  .foot-label {
    font-size: 11px;
    font-weight: 600;
    fill: #9ea7b6;
  }
  .foot-delta {
    font-size: 12.5px;
    font-weight: 700;
    fill: #16a34a;
  }
  .pill-running {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.06em;
    fill: #c92912;
  }
  .medal {
    font-size: 13px;
  }
  .chip-code {
    font-size: 10.5px;
    font-weight: 700;
    fill: #36547f;
    font-family: ui-monospace, Menlo, monospace;
  }
  .chip-label {
    font-size: 9.5px;
    font-weight: 600;
    fill: #9ea7b6;
  }
  .row-deep {
    font-size: 12px;
    font-weight: 700;
    fill: #2055a0;
  }
  .row-you {
    font-size: 12px;
    font-weight: 600;
    fill: #0269fb;
  }
  .row-mut {
    font-size: 12px;
    font-weight: 500;
    fill: #363636;
  }
  .score-you,
  .score-mut {
    font-size: 12px;
    font-weight: 700;
    fill: #363636;
    font-variant-numeric: tabular-nums;
  }
  .wire-path {
    fill: none;
    stroke: #c9d8ef;
    stroke-width: 1.6;
    stroke-dasharray: 5 6;
    opacity: 0;
    transition: opacity 0.5s;
  }
  .dot {
    opacity: 0;
    transition: opacity 0.35s;
  }
  .dot-blue {
    fill: #0269fb;
  }
  .dot-ok {
    fill: #14934a;
  }

  .task-line {
    font-size: 9.5px;
    font-weight: 700;
    fill: #36547f;
  }
  .task-fly-label {
    font-size: 9.5px;
    font-weight: 700;
    fill: #ffffff;
    font-family: inherit;
  }
  .task-fly {
    opacity: 0;
    transition: opacity 0.35s;
  }
  .phase-2 .task-fly {
    opacity: 1;
  }
  .g-task,
  .g-you,
  .g-rivals,
  .g-hub,
  .g-judges,
  .g-board,
  .g-code,
  .judge-row,
  .flash,
  .popchip {
    opacity: 0;
    transition:
      opacity 0.55s ease,
      transform 0.55s ease;
  }
  .seen-1 .g-you,
  .seen-1 .g-hub {
    opacity: 1;
  }
  .g-you,
  .g-rivals {
    transform: translateX(-14px);
  }
  .seen-1 .g-you {
    transform: translateX(0);
  }
  .seen-1 .g-rivals {
    opacity: 1;
    transform: translateX(0);
    transition-delay: 1.1s;
  }
  .seen-2 .g-rivals {
    transition-delay: 0s;
  }
  .seen-2 .g-code {
    opacity: 1;
  }
  .seen-2 .g-task {
    opacity: 1;
    transition-delay: 0s;
  }
  .phase-2 .g-task {
    transition-delay: 1s;
  }
  .phase-2 .wire-probe,
  .seen-2 .wire-probe {
    opacity: 1;
  }
  .seen-4 .g-judges,
  .phase-4 .g-judges {
    opacity: 1;
  }
  .seen-5 .g-board,
  .phase-5 .g-board {
    opacity: 1;
  }

  /* typed commands (phase 1) */
  .cmd-type {
    clip-path: inset(-2px 100% -2px 0);
  }
  .phase-1 .cmd-you.cmd-type {
    animation: typecmd 1.1s steps(26) 0.35s forwards;
  }
  .phase-1 .cmd-r1.cmd-type {
    animation: typecmd 0.8s steps(16) 1.5s forwards;
  }
  .phase-1 .cmd-r2.cmd-type {
    animation: typecmd 0.8s steps(16) 1.9s forwards;
  }
  .seen-2 .cmd-type {
    clip-path: inset(-2px 0% -2px 0);
  }
  @keyframes typecmd {
    to {
      clip-path: inset(-2px 0% -2px 0);
    }
  }

  /* hub pulse */
  .pulse {
    fill: none;
    stroke: #0269fb;
    stroke-width: 2;
    opacity: 0;
    transform-box: fill-box;
    transform-origin: center;
  }
  .phase-1 .pulse,
  .phase-3 .pulse {
    animation: pulse 2.2s ease-out infinite;
  }
  .pulse.p2 {
    animation-delay: 1.1s !important;
  }
  @keyframes pulse {
    0% {
      transform: scale(0.72);
      opacity: 0.65;
    }
    80% {
      transform: scale(1.45);
      opacity: 0;
    }
    100% {
      opacity: 0;
    }
  }

  /* phase 2: code lines type in */
  .code-line {
    transform-box: fill-box;
    transform-origin: left center;
    transform: scaleX(0);
  }
  .phase-2 .code-line {
    animation: type 0.5s ease-out forwards;
  }
  .seen-3 .code-line,
  .seen-4 .code-line,
  .seen-5 .code-line {
    transform: scaleX(1);
  }
  .phase-2 .cl1 {
    animation-delay: 1.6s;
  }
  .phase-2 .cl2 {
    animation-delay: 2s;
  }
  .phase-2 .cl3 {
    animation-delay: 2.4s;
  }
  .phase-2 .cl4 {
    animation-delay: 2.8s;
  }
  @keyframes type {
    to {
      transform: scaleX(1);
    }
  }
  .cursor {
    opacity: 0;
  }
  .phase-2 .cursor {
    opacity: 1;
    animation: blink 0.9s steps(1) infinite;
  }
  @keyframes blink {
    50% {
      opacity: 0;
    }
  }

  /* phase 3: probes + wires + flashes */
  .phase-3 .wire-probe,
  .seen-3 .wire-probe {
    opacity: 1;
  }
  .phase-4 .wire-judge,
  .seen-4 .wire-judge {
    opacity: 1;
  }
  .phase-5 .wire-board,
  .seen-5 .wire-board {
    opacity: 1;
  }
  .phase-3 .dot-probe {
    opacity: 1;
  }
  .phase-4 .dot-judge {
    opacity: 1;
  }
  .phase-5 .dot-board {
    opacity: 1;
  }
  .phase-3 .flash {
    animation: flashpop 2.4s ease-out infinite;
  }
  .flash.f2 {
    animation-delay: 0.5s !important;
  }
  .flash.f3 {
    animation-delay: 1s !important;
  }
  @keyframes flashpop {
    0%,
    34% {
      opacity: 0;
      transform: translateY(5px);
    }
    44%,
    82% {
      opacity: 1;
      transform: translateY(0);
    }
    95%,
    100% {
      opacity: 0;
      transform: translateY(-4px);
    }
  }

  /* phase 4: judge rows pop in sequence */
  .phase-4 .judge-row {
    animation: rowpop 0.5s ease-out forwards;
  }
  .phase-4 .jr2 {
    animation-delay: 0.7s;
  }
  .phase-4 .jr3 {
    animation-delay: 1.4s;
  }
  .seen-5 .judge-row {
    opacity: 1;
  }
  @keyframes rowpop {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  /* phase 5: leaderboard rows land one by one, rating chip pops */
  .board-row {
    opacity: 0;
  }
  .phase-5 .board-row {
    animation: rowpop 0.5s ease-out forwards;
  }
  .phase-5 .br2 {
    animation-delay: 0.35s;
  }
  .phase-5 .br3 {
    animation-delay: 0.7s;
  }
  .seen-5 .board-row {
    opacity: 1;
  }
  .phase-5 .popchip {
    opacity: 1;
    transform: translateY(0);
    transition-delay: 0.9s;
  }
  .popchip {
    transform: translateY(6px);
  }

  @media (prefers-reduced-motion: reduce) {
    .pulse,
    .flash,
    .cursor,
    .dot,
    .cmd-type,
    .code-line,
    .judge-row,
    .board-row {
      animation: none !important;
    }
    .seen-5 .cmd-type {
      clip-path: inset(-2px 0% -2px 0);
    }
    .seen-5 .flash,
    .seen-5 .wire-path,
    .seen-5 .dot {
      opacity: 1;
    }
  }
</style>
