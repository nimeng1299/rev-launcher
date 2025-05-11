import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";
import styles from "./TitleBar.module.css";
import appStyles from "./app.module.css";
import {
  MenuItem,
  MenuList,
  Text,
  ToolbarButton,
} from "@fluentui/react-components";
import {
  ArrowMinimizeRegular,
  DismissRegular,
  ListRegular,
  PeopleRegular,
  SquareRegular,
} from "@fluentui/react-icons";
import { Window } from "@tauri-apps/api/window";
import { GamesRegular } from "@fluentui/react-icons/fonts";

function App() {
  const appWindow = new Window("main");
  const [greetMsg, setGreetMsg] = useState("");
  const [name, setName] = useState("");
  const [isMaximized, setIsMaximized] = useState(false);

  const handleMinimize = () => appWindow.minimize();
  const handleToggleMaximize = async () => {
    const newState = !isMaximized;
    await appWindow.toggleMaximize();
    setIsMaximized(newState);
  };
  const handleClose = () => appWindow.close();

  async function greet() {
    // Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
    setGreetMsg(await invoke("greet", { name }));
  }

  return (
    <main className="container">
      <div data-tauri-drag-region className={styles.titleBar}>
        <div className={styles.leftSection}>
          <ToolbarButton
            className={styles.toolbarButton}
            icon={<ListRegular />}
          />
          <Text>Rev Launcher</Text>
        </div>
        <div data-tauri-drag-region />
        <div>
          <ToolbarButton
            aria-label="Minimize"
            icon={<ArrowMinimizeRegular />}
            onClick={handleMinimize}
          />
          <ToolbarButton
            aria-label={isMaximized ? "Restore" : "Maximize"}
            icon={isMaximized ? <SquareRegular /> : <SquareRegular />}
            onClick={handleToggleMaximize}
          />
          <ToolbarButton
            aria-label="Close"
            icon={<DismissRegular />}
            onClick={handleClose}
          />
        </div>
      </div>

      <div className={appStyles.layoutContainer}>
        <MenuList className={appStyles.sidebar}>
          <MenuItem icon={<PeopleRegular />}>Account</MenuItem>
          <MenuItem icon={<GamesRegular />}>Games</MenuItem>
        </MenuList>

        <div className={appStyles.mainContent}>
          <form
            className="row"
            onSubmit={(e) => {
              e.preventDefault();
              greet();
            }}
          >
            <input
              id="greet-input"
              onChange={(e) => setName(e.currentTarget.value)}
              placeholder="Enter a name..."
            />
            <button type="submit">Greet</button>
          </form>
          <p>{greetMsg}</p>
        </div>
      </div>
    </main>
  );
}

export default App;
