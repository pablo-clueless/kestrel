import { create } from "zustand";

export type SidebarTab = "collections" | "history";

export type ProfileTab = "account" | "security" | "appearance" | "notifications";

interface LayoutState {
  sidebarOpen: boolean;
  sidebarTab: SidebarTab;
  /** Collections open in the sidebar. Any number, none included; separate from the active one. */
  expandedCollections: string[];
  toggleSidebar: () => void;
  setSidebarTab: (tab: SidebarTab) => void;
  expandCollection: (id: string) => void;
  collapseCollection: (id: string) => void;
  collapseAllCollections: () => void;
}

export const useLayoutStore = create<LayoutState>()((set) => ({
  sidebarOpen: true,
  sidebarTab: "collections",
  toggleSidebar: () => set((s) => ({ sidebarOpen: !s.sidebarOpen })),
  setSidebarTab: (sidebarTab) => set({ sidebarTab }),
  expandedCollections: [],
  expandCollection: (id) =>
    set((s) =>
      s.expandedCollections.includes(id)
        ? s
        : { expandedCollections: [...s.expandedCollections, id] },
    ),
  collapseCollection: (id) =>
    set((s) => ({ expandedCollections: s.expandedCollections.filter((c) => c !== id) })),
  collapseAllCollections: () => set({ expandedCollections: [] }),
}));
