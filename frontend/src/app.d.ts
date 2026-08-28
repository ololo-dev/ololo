// See https://kit.svelte.dev/docs/types#app
declare global {
  namespace App {
    interface Locals {
      isAuthenticated: boolean;
      userId: string | null;
    }
    interface PageData {
      isAuthenticated: boolean;
      isAdmin: boolean;
      allowProjectCreation?: boolean;
    }
    // interface Error {}
    // interface Platform {}
  }
}

// mdsvex: treat .md files as Svelte components
declare module "*.md" {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const component: any;
  export default component;
  export const metadata: Record<string, unknown>;
}

export {};
