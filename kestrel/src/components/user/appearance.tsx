import { TabPanel } from "../shared";

interface Props {
  selected: string;
}

export const Appearance = ({ selected }: Props) => {
  return (
    <TabPanel selected={selected} value="appearance">
      <div className="bg-background flex h-[calc(100%-36px)] flex-col items-center justify-center p-5">
        Appearance
      </div>
    </TabPanel>
  );
};
