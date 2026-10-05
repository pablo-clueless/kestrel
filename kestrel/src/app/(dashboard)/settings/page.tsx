const Page = () => {
  return (
    <div className="h-full overflow-y-auto">
      <div className="h-fulkl flex w-full flex-col gap-6 p-5">
        <div>
          <h1 className="text-lg font-semibold">Settings</h1>
          <p className="text-muted-foreground text-sm">Workspace and engine settings.</p>
        </div>
        <div className="bg-card h-[calc(100%-48px)] overflow-hidden rounded-xs border">
          Nothing to configure yet.
        </div>
      </div>
    </div>
  );
};

export default Page;
