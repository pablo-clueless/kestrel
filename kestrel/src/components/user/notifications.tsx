import { TabPanel } from "../shared";

interface Props {
  selected: string;
}

export const Notifications = ({ selected }: Props) => {
  return (
    <TabPanel selected={selected} value="notifications">
      <div className="bg-background flex h-[calc(100%-36px)] flex-col gap-4 overflow-y-auto p-5">
        Notifications
      </div>
    </TabPanel>
  );
};
