"use client";

import { zodResolver } from "@hookform/resolvers/zod";
import { useRouter } from "next/navigation";
import { useForm } from "react-hook-form";
import { z } from "zod";

import { PASSWORD_MESSAGE, PASSWORD_REGEX } from "@/config/string";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

const schema = z.object({
  email: z.string().email("Invalid email address").min(1, "Email is required"),
  password: z
    .string()
    .min(8, "Password must be at least 8 characters")
    .regex(PASSWORD_REGEX, PASSWORD_MESSAGE),
});

type FormValues = z.infer<typeof schema>;

const defaultValues: FormValues = {
  email: "",
  password: "",
};

const Page = () => {
  const router = useRouter();

  const { handleSubmit } = useForm({
    defaultValues,
    resolver: zodResolver(schema),
  });

  const onSubmit = (values: FormValues) => {
    router.push("/workspace");
  };

  return (
    <div className="flex flex-col items-center gap-6">
      <div className="text-center">
        <p className="text-xl font-bold">Welcome</p>
        <p className="text-muted-foreground text-sm">Sign in to continue</p>
      </div>
      <form className="space-y-4" onSubmit={handleSubmit(onSubmit)}>
        <Button type="submit">Continue</Button>
      </form>
    </div>
  );
};

export default Page;
