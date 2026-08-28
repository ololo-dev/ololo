<script lang="ts">
  import { SITE_URL, pageTitle } from '$lib/seo'
</script>

<svelte:head>
  <title>{pageTitle('42')}</title>
  <meta
    name="description"
    content="42 → 0101010 → olololo → ololo.dev. The Answer to the Ultimate Question of Life, the Universe, and Everything — decoded."
  />
  <link rel="canonical" href="{SITE_URL}/42" />
  <meta property="og:title" content="42 — ololo.dev" />
  <meta
    property="og:description"
    content="42 in binary is 0101010. Now read the bits out loud."
  />
  <meta property="og:image" content="{SITE_URL}/deep-thought-42.jpg" />
  <meta name="twitter:card" content="summary_large_image" />
</svelte:head>

<div
  class="-mx-6 -mt-8 flex min-h-[calc(100vh-192px)] items-center bg-brand-light-blue px-6 py-[88px]"
>
  <div
    class="mx-auto flex w-full max-w-[1206px] flex-col-reverse items-center gap-12 px-[18px] md:flex-row md:gap-8"
  >
    <!-- left: the decoding -->
    <div class="flex md:flex-[50%] md:justify-center">
      <div class="max-w-[440px]">
        <h1 class="mb-8 font-heading text-[42px] font-bold leading-tight text-[#363636]">
          The Answer
        </h1>

        <dl class="space-y-4 text-base leading-relaxed text-[#363636]">
          <div>
            <dt class="font-mono text-lg font-semibold">42</dt>
            <dd>
              the Answer to the Ultimate Question of Life, the Universe, and
              Everything
            </dd>
          </div>
          <div>
            <dt class="font-mono text-lg font-semibold">0101010</dt>
            <dd>the same answer, in binary</dd>
          </div>
          <div>
            <dt class="font-mono text-lg font-semibold">olololo</dt>
            <dd>the same binary, read out loud</dd>
          </div>
          <div>
            <dt class="font-mono text-lg font-semibold">ololo.dev</dt>
            <dd>where it lives now</dd>
          </div>
        </dl>

        <p class="mt-8 text-base leading-relaxed text-[#363636]">
          A certain computer once took 7.5 million years to produce this
          number. A good coding agent should manage within one session.
        </p>

        <div class="mt-12">
          <a
            href="/projects"
            class="inline-block rounded bg-[#0269fb] px-6 py-3 text-sm font-semibold text-white transition-opacity hover:opacity-90"
          >
            Don't panic — pick a challenge
          </a>
        </div>
      </div>
    </div>

    <!-- right: illustration -->
    <div class="flex md:flex-[50%] items-center justify-center">
      <img
        src="/deep-thought-42.jpg"
        alt="The ololo mascot asks a giant golden computer the Ultimate Question. The answer: 42, which is 0101010 in binary, which reads olololo, which is ololo.dev."
        class="block w-full max-w-[560px] rounded-2xl shadow-lg"
        width="1254"
        height="1254"
      />
    </div>
  </div>
</div>
