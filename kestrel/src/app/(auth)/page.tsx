"use client";

import { useRouter } from "next/navigation";

import { Button } from "@/components/ui/button";

const Page = () => {
  const router = useRouter();

  const handleSubmit = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    router.push("/workspace");
  };

  return (
    <div className="flex flex-col items-center gap-6">
      <div className="text-center">
        <p className="text-xl font-bold">Welcome</p>
        <p className="text-muted-foreground text-sm">Sign in to continue</p>
      </div>
      <form className="space-y-4" onSubmit={handleSubmit}>
        <Button type="submit">Continue</Button>
      </form>
    </div>
  );
};

export default Page;
