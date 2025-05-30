import { Label, Select, useId } from "@fluentui/react-components";
import { SettingProps } from "./SettingProps";

function JavaSetting(props: SettingProps) {
  const selectJavaId = useId("select-java-version");

  return (
    <div>
      <Label htmlFor={selectJavaId} style={{ textAlignLast: "left" }}>
        Java 版本：
      </Label>
      <Select id={selectJavaId}>
        <option>8.0</option>
        <option>22</option>
      </Select>
    </div>
  );
}
export default JavaSetting;
