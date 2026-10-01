import { create } from "zustand";

import { UserInfo } from "@/types/engine/UserInfo";

interface AuthState {
  user: UserInfo | null;

  setUser: (user: UserInfo) => void;
}

export const useAuthStore = create<AuthState>()((set) => ({
  user: null,

  setUser: (user) => set({ user }),
}));
