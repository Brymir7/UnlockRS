import os

def count_lines_in_file(filepath):
    with open(filepath, 'r', encoding='utf-8') as file:
        return sum(1 for _ in file)

def count_lines_in_project(directory):
    total_lines = 0

    for root, _, files in os.walk(directory):
        for file in files:
            if file.endswith('.rs'):
                filepath = os.path.join(root, file)
                file_lines = count_lines_in_file(filepath)
                total_lines += file_lines
                print(f"{filepath}: {file_lines} lines")

    return total_lines

if __name__ == "__main__":
    project_directory = "."  # Specify your Cargo project directory
    total_lines = count_lines_in_project(project_directory)
    print(f"Total number of lines in project: {total_lines}")
