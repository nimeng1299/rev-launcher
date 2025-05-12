import {
  Button,
  Menu,
  MenuTrigger,
  MenuButton,
  MenuPopover,
  MenuList,
  MenuItem,
} from "@fluentui/react-components";
import styles from "./Games.module.css";
import { PlayRegular, SettingsRegular } from "@fluentui/react-icons";
import { useState } from "react";

function Games() {
  const [expanded, setExpanded] = useState(false);

  return (
    <div>
      <div className={styles.menu}>
        <Menu open={expanded}>
          <MenuTrigger>
            <MenuButton
              onMouseEnter={() => setExpanded(true)}
              onMouseLeave={() => setExpanded(false)}
            >
              Example
            </MenuButton>
          </MenuTrigger>
          <MenuPopover
            onMouseEnter={() => setExpanded(true)}
            onMouseLeave={() => setExpanded(false)}
          >
            <MenuList
              style={{ display: "flex", flexDirection: "column", gap: "8px" }}
            >
              <MenuItem>Item a</MenuItem>
              <MenuItem>Item b</MenuItem>
            </MenuList>
          </MenuPopover>
        </Menu>
      </div>
    </div>
  );
}

export default Games;
