const Page = () => {
  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex max-w-3xl flex-col gap-6 p-5">
        <div>
          <h1 className="text-lg font-semibold">Settings</h1>
          <p className="text-muted-foreground text-sm">Workspace and engine settings.</p>
        </div>
        <div className="bg-card text-muted-foreground rounded-xs border p-5 text-sm">
          Nothing to configure yet.
        </div>
      </div>
    </div>
  );
};

export default Page;
