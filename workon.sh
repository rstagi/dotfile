#!/bin/bash
# Set the directory you want to work in
PROJECT_DIR="$1"
# If no directory is provided, use the current one
if [ -z "$PROJECT_DIR" ]; then
  PROJECT_DIR="."
fi

# Get the folder name for session naming
if [ "$PROJECT_DIR" = "." ]; then
  SESSION_NAME=$(basename "$(pwd)")
else
  SESSION_NAME=$(basename "$PROJECT_DIR")
fi

# Function to handle name conflicts
handle_name_conflict() {
  local name="$1"
  local type="$2"
  local check_cmd="$3"
  local action_cmd="$4"
  local action_name="$5"
  
  if eval "$check_cmd" 2>/dev/null; then
    echo "$type '$name' already exists." >&2
    echo "Would you like to:" >&2
    echo "1) $action_name existing $type" >&2
    echo "2) Create new $type with incremented name" >&2
    read -p "Enter your choice (1 or 2): " choice >&2
    
    case $choice in
      1)
        eval "$action_cmd"
        exit 0
        ;;
      2)
        # Find next available name
        counter=2
        local check_incremented_cmd
        while true; do
          check_incremented_cmd="${check_cmd//$name/${name}_$counter}"
          if ! eval "$check_incremented_cmd" 2>/dev/null; then
            break
          fi
          counter=$((counter + 1))
        done
        echo "${name}_$counter"
        ;;
      *)
        echo "Invalid choice. Exiting." >&2
        exit 1
        ;;
    esac
  else
    echo "$name"
  fi
}

# Check if session already exists and handle it
if [ -z "$TMUX" ]; then
  SESSION_NAME=$(handle_name_conflict "$SESSION_NAME" "Session" "tmux has-session -t $SESSION_NAME" "tmux attach-session -t $SESSION_NAME" "Attach to")
fi

# Check if we're already in a tmux session
if [ -n "$TMUX" ]; then
  # We're in tmux, handle window creation
  WINDOW_NAME="$SESSION_NAME"
  
  # Check if window already exists
  if tmux list-windows -F '#{window_name}' | grep -q "^$WINDOW_NAME$"; then
    echo "Window '$WINDOW_NAME' already exists." >&2
    echo "Would you like to:" >&2
    echo "1) Switch to existing window" >&2
    echo "2) Create new window with incremented name" >&2
    read -p "Enter your choice (1 or 2): " choice >&2
    
    case $choice in
      1)
        tmux select-window -t "$WINDOW_NAME"
        exit 0
        ;;
      2)
        # Find next available window name
        counter=2
        while tmux list-windows -F '#{window_name}' | grep -q "^${WINDOW_NAME}_$counter$"; do
          counter=$((counter + 1))
        done
        WINDOW_NAME="${WINDOW_NAME}_$counter"
        ;;
      *)
        echo "Invalid choice. Exiting." >&2
        exit 1
        ;;
    esac
  fi
  
  tmux new-window -c "$PROJECT_DIR" -n "$WINDOW_NAME"
  WINDOW_ID=$(tmux display-message -p '#I')
  TARGET="$WINDOW_ID"
else
  # We're not in tmux, create a new session
  tmux new-session -d -s "$SESSION_NAME" -c "$PROJECT_DIR" -x "$(tput cols)" -y "$(tput lines)"
  TARGET="$SESSION_NAME:1"
fi

# Create the desired layout:
# Left column (20% width): tmuxai (30% height) + terminal (70% height)  
# Right column (80% width): vim (45%) + claude (35%)
tmux split-window -h -p 80 -t "$TARGET.1"
tmux split-window -h -p 44 -t "$TARGET.2"  
tmux split-window -v -p 70 -t "$TARGET.1"

# Send commands to each pane
tmux send-keys -t "$TARGET.1" 'tmuxai' C-m
tmux send-keys -t "$TARGET.3" 'vim .' C-m
tmux send-keys -t "$TARGET.4" 'claude' C-m

# Select the bottom left pane (now containing vim) to be the active one initially
tmux select-pane -t "$TARGET.2"

# If we created a new session, attach to it
if [ -z "$TMUX" ]; then
  tmux attach-session -t "$SESSION_NAME"
fi

