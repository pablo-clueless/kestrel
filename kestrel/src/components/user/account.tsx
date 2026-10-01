import { TabPanel } from "../shared";

interface Props {
  selected: string;
}

export const Account = ({ selected }: Props) => {
  return (
    <TabPanel selected={selected} value="account">
      <div className="bg-background p-5">Account</div>
    </TabPanel>
  );
};
