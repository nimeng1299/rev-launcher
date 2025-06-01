import { Label, Select, useId } from "@fluentui/react-components";
import { SettingProps } from "./SettingProps";
import { invoke } from "@tauri-apps/api/core";

import { useEffect, useState } from "react";

function display(version) {
  return version["path"] + " - " + version["version"]["value"];
}

function JavaSetting(props: SettingProps) {
  const [javaVersions, setJavaVersions] = useState<string[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  useEffect(() => {
    const fetchJavaVersions = async () => {
      try {
        const list = await invoke("get_setting_value", {
          id: -1,
          itemName: "java",
        });
        setJavaVersions(
          Array.isArray(list["versions"]) ? list["versions"] : [],
        );
        setSelectedIndex(list["select"]);
        console.log("Java 版本列表:", javaVersions);
      } catch (error) {
        console.error("获取 Java 版本失败:", error);
        setJavaVersions([]);
      }
    };

    fetchJavaVersions();
  }, []);
  const selectJavaId = useId("select-java-version");

  const handleSelectChange = async (e) => {
    const value = e.target.value;

    if (value === "add") {
      try {
        const filters = [["java", ["exe"]]];
        const directory = "/";
        const result = await invoke("file_dialog", {
          filter: filters,
          setDirectory: directory,
        });
        // 处理返回结果
        if (result) {
          // 这里可以添加逻辑把它加到列表里
          console.log("Selected path:", result);
        } else {
          console.log("User cancelled the dialog");
        }
      } catch (error) {
        console.error("Error opening file dialog:", error);
      }

      return;
    }

    setSelectedIndex(value);
  };

  return (
    <div style={{ whiteSpace: "pre-wrap" }}>
      <Label htmlFor={selectJavaId} style={{ textAlignLast: "left" }}>
        Java 版本：
      </Label>

      <Select
        id={selectJavaId}
        value={selectedIndex}
        onChange={handleSelectChange}
      >
        {javaVersions.map((version, i) => (
          <option value={i}>{display(version)}</option>
        ))}

        <option value="add">添加...</option>
      </Select>
    </div>
  );
}
export default JavaSetting;
