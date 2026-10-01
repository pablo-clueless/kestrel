import { TabPanel } from "../shared";

interface Props {
  selected: string;
}

export const Appearance = ({ selected }: Props) => {
  return (
    <TabPanel selected={selected} value="appearance">
      <div className="bg-background p-5">Appearance</div>
    </TabPanel>
  );
};
