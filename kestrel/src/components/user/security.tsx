import { TabPanel } from "../shared";

interface Props {
  selected: string;
}

export const Security = ({ selected }: Props) => {
  return (
    <TabPanel selected={selected} value="security">
      <div className="bg-background p-5">Security</div>
    </TabPanel>
  );
};
