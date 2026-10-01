import { TabPanel } from "../shared";

interface Props {
  selected: string;
}

export const Notifications = ({ selected }: Props) => {
  return (
    <TabPanel selected={selected} value="notifications">
      <div className="bg-background p-5">Notifications</div>
    </TabPanel>
  );
};
