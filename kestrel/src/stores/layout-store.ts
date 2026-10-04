import { create } from "zustand";

export type SidebarTab = "collections" | "history";

export type ProfileTab = "account" | "security" | "appearance" | "notifications";

interface LayoutState {
  sidebarOpen: boolean;
  sidebarTab: SidebarTab;
  /** Collections open in the sidebar. Any number, none included; separate from the active one. */
  expandedCollections: string[];
  /** Groups closed in the sidebar, as `groupKey(collectionId, group)`. Groups start open. */
  collapsedGroups: string[];
  toggleSidebar: () => void;
  setSidebarTab: (tab: SidebarTab) => void;
  expandCollection: (id: string) => void;
  collapseCollection: (id: string) => void;
  collapseAllCollections: () => void;
  toggleGroup: (collectionId: string, group: string) => void;
  expandGroup: (collectionId: string, group: string) => void;
}

/** A group's key: its collection's id (a UUID, so no `/`), then the name, which may contain `/`. */
export const groupKey = (collectionId: string, group: string) => `${collectionId}/${group}`;

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
  collapsedGroups: [],
  toggleGroup: (collectionId, group) =>
    set((s) => {
      const key = groupKey(collectionId, group);
      return {
        collapsedGroups: s.collapsedGroups.includes(key)
          ? s.collapsedGroups.filter((k) => k !== key)
          : [...s.collapsedGroups, key],
      };
    }),
  expandGroup: (collectionId, group) =>
    set((s) => ({
      collapsedGroups: s.collapsedGroups.filter((k) => k !== groupKey(collectionId, group)),
    })),
}));
