import { create } from "zustand";

export type SidebarTab = "collections" | "history";

export type ProfileTab = "account" | "security" | "appearance" | "notifications";

interface LayoutState {
  sidebarOpen: boolean;
  sidebarTab: SidebarTab;
  toggleSidebar: () => void;
  setSidebarTab: (tab: SidebarTab) => void;
}

export const useLayoutStore = create<LayoutState>()((set) => ({
  sidebarOpen: true,
  sidebarTab: "collections",
  toggleSidebar: () => set((s) => ({ sidebarOpen: !s.sidebarOpen })),
  setSidebarTab: (sidebarTab) => set({ sidebarTab }),
}));
