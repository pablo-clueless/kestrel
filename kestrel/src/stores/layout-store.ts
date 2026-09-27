import { create } from "zustand";

interface LayoutState {
  sidebarOpen: boolean;
  toggleSidebar: () => void;
}

export const useLayoutStore = create<LayoutState>()((set) => ({
  sidebarOpen: true,
  toggleSidebar: () => set((s) => ({ sidebarOpen: !s.sidebarOpen })),
}));
