"use client";

import { useRouter } from "next/navigation";
import { useEffect } from "react";

/** `/admin` itself has nothing on it: the overview is the admin home. (A static export can't redirect
 * on the server.) */
const Page = () => {
  const router = useRouter();
  useEffect(() => router.replace("/admin/overview"), [router]);
  return null;
};

export default Page;
