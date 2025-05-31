import { Label, Select, useId } from "@fluentui/react-components";
import { SettingProps } from "./SettingProps";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
function JavaSetting(props: SettingProps) {
  const [javaVersions, setJavaVersions] = useState<string[]>([]);
  useEffect(() => {
    const fetchJavaVersions = async () => {
      try {
        const list = await invoke("get_setting_value", {
          id: -1,
          itemName: "java",
        });

        setJavaVersions(Array.isArray(list) ? list : []);
      } catch (error) {
        console.error("获取 Java 版本失败:", error);
        setJavaVersions([]);
      }
    };

    fetchJavaVersions();
  }, []);
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
