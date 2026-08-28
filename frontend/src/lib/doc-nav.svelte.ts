interface DocItem {
  slug: string;
  title: string;
}

interface DocSection {
  id: string;
  label: string;
  items: DocItem[];
}

class DocNavStore {
  sections = $state<DocSection[]>([]);

  setSections(data: DocSection[]): void {
    this.sections = data;
  }

  reset(): void {
    this.sections = [];
  }
}

export const docNav = new DocNavStore();
export type { DocItem, DocSection };
