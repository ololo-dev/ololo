<script lang="ts">
  const navLinks = [
    { label: 'Projects',      href: '/projects'      },
    { label: 'Install',       href: '/#install'      },
    { label: 'Documentation', href: '/documentation' },
  ];
</script>

<footer class="relative z-10 bg-[#2055a0] py-6">
  <div class="mx-auto flex max-w-7xl items-center justify-between px-6
              max-sm:flex-col max-sm:gap-8">

    <!-- Logo -->
    <a href="/" aria-label="ololo.dev home">
      <img src="/logo-footer.svg" alt="ololo.dev" class="h-10 w-10" />
    </a>

    <!-- Nav -->
    <nav aria-label="Footer navigation">
      <ul class="flex flex-wrap items-center gap-8 max-sm:flex-col max-sm:items-center max-sm:gap-5">
        {#each navLinks as link (link.href)}
          <li>
            <a
              href={link.href}
              class="font-heading font-semibold text-white/80 transition-colors hover:text-white"
            >
              {link.label}
            </a>
          </li>
        {/each}
      </ul>
    </nav>

  </div>

  <div class="mx-auto mt-5 flex max-w-7xl flex-wrap items-center justify-between gap-3 border-t border-white/15 px-6 pt-4">
    <p class="text-xs text-white/50">© 2026 ololo.dev</p>
  </div>
</footer>
